// Codex 账号测试：Identity, token import, reauthorization and refresh behavior。
// 测试与生产实现共享 super 作用域，验证真实持久化和运行态行为。
    use super::{
        authority_projection_dirs_for_account, build_account_storage_id,
        build_agent_identity_account_draft, build_auth_file_value,
        build_legacy_agent_identity_account_id, clear_client_auth_observation,
        clear_retired_app_server_preflight_reauth,
        decode_jwt_payload_value, detect_auth_file_plan_type_from_path,
        ensure_managed_account_fresh, extract_codex_import_candidate_from_value,
        extract_codex_tokens_from_value, extract_user_info,
        force_refresh_managed_account_after_observed, format_account_switch_error,
        format_refresh_error_for_user, get_accounts_dir, get_accounts_storage_path,
        get_current_account_from_loaded, import_from_json, is_loopback_http_base_url,
        is_managed_auth_refresh_due, is_pending_oauth_account, list_accounts_checked, load_account,
        load_account_index, looks_like_sub2api_export, managed_account_runtime_tokens_need_refresh,
        merge_existing_auth_file_value, now_timestamp, parse_agent_identity_from_value,
        parse_auth_file_last_refresh, parse_codex_account_compat, parse_line_delimited_json_values,
        prepare_account_for_injection_from_auth_dir, read_api_provider_from_config_toml,
        read_experimental_model_definitions, read_managed_projection_from_dir,
        read_quick_config_from_config_toml, remove_accounts, resolve_api_provider_config,
        save_account, save_account_index, should_accept_authority_snapshot,
        sync_account_from_auth_dir, sync_account_from_authority_dir_if_current,
        sync_api_key_account_from_local_state, sync_api_key_provider_accounts,
        sync_managed_projection_from_auth_dir, try_parse_pending_oauth_delimited_line,
        update_account_instance_access, update_api_key_credentials, upsert_account,
        upsert_account_for_reauth, upsert_account_from_access_token,
        upsert_account_from_access_token_with_hints, upsert_account_from_auth_tokens,
        upsert_agent_identity_account, upsert_api_key_account, validate_api_key_credentials,
        write_account_bundle_to_dir, write_api_key_bearer_provider_override_to_config_toml,
        write_api_provider_to_config_toml, write_auth_file_to_dir, write_managed_projection_to_dir,
        write_quick_config_to_config_toml, write_quick_config_to_config_toml_with_default,
        ApiProviderConfig, CodexAccessTokenImportHints, CodexAccountGroupRecord, CodexAccountIndex,
        CodexAccountSummary, CodexAuthFile, CodexAuthTokens, CodexGroupQuotaRefreshPolicy,
        CodexJsonImportCandidate, LocalCodexOAuthSnapshot, CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION,
        CODEX_AUTHORIZATION_STATUS_PENDING, CODEX_AUTH_PROJECTION_VERSION,
        CODEX_AUTO_COMPACT_DEFAULT_LIMIT, CODEX_CONTEXT_WINDOW_1M_VALUE,
        CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
        CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE, CODEX_IMAGEGEN_ACTOR_HEADER,
        CODEX_IMAGEGEN_ACTOR_HEADER_VALUE, CODEX_IMAGE_MODEL_ID, CODEX_RUNTIME_MODEL_PROVIDER_ID,
    };
    use crate::models::codex::{
        CodexAccount, CodexAgentIdentity, CodexApiModelMapping, CodexApiProviderMode,
        CodexExperimentalModelDefinition, CodexTokens,
    };
    use crate::models::{InstanceLaunchMode, InstanceProfile, InstanceStore};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};
    use toml_edit::Document;

    fn agent_identity_private_key() -> String {
        let rng = ring::rand::SystemRandom::new();
        let key = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .expect("generate Agent Identity private key");
        base64::engine::general_purpose::STANDARD.encode(key.as_ref())
    }

    fn sub2api_agent_identity_v1_private_key() -> String {
        let mut der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        der.extend(1u8..=32u8);
        base64::engine::general_purpose::STANDARD.encode(der)
    }

    #[test]
    fn parses_and_projects_agent_identity_auth_json() {
        let raw = serde_json::json!({
            "auth_mode": "agentIdentity",
            "type": "codex",
            "account_id": "team-test",
            "user_id": "user-test",
            "agent_identity": {
                "auth_mode": "agentIdentity",
                "agent_runtime_id": "runtime-test",
                "agent_private_key": agent_identity_private_key(),
                "task_id": "task-test",
                "account_id": "team-test",
                "chatgpt_account_id": "team-test",
                "chatgpt_user_id": "user-test",
                "email": "agent@example.com",
                "plan_type": "plus",
                "chatgpt_account_is_fedramp": true
            }
        });
        let identity = parse_agent_identity_from_value(&raw)
            .expect("parse Agent Identity")
            .expect("Agent Identity should be detected");
        let account = super::build_agent_identity_account_draft(identity)
            .expect("build Agent Identity account");
        assert!(account.is_agent_identity_auth());
        assert_eq!(account.account_id.as_deref(), Some("team-test"));
        assert!(account
            .agent_identity
            .as_ref()
            .is_some_and(|identity| identity.chatgpt_account_is_fedramp));
        let projected = build_auth_file_value(&account).expect("project auth.json");
        assert_eq!(
            projected
                .get("auth_mode")
                .and_then(serde_json::Value::as_str),
            Some("agentIdentity")
        );
        assert_eq!(
            projected.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert_eq!(
            projected
                .pointer("/agent_identity/task_id")
                .and_then(serde_json::Value::as_str),
            Some("task-test")
        );
        assert!(projected.get("tokens").is_none());
    }

    #[test]
    fn parses_agent_identity_camel_case_root_format() {
        let raw = serde_json::json!({
            "authMode": "agentIdentity",
            "agentRuntimeId": "runtime-camel",
            "agentPrivateKey": agent_identity_private_key(),
            "accountId": "team-camel",
            "chatgptUserId": "user-camel"
        });
        let identity = parse_agent_identity_from_value(&raw)
            .expect("parse camel-case Agent Identity")
            .expect("Agent Identity should be detected");
        assert_eq!(identity.agent_runtime_id, "runtime-camel");
        assert_eq!(identity.account_id, "team-camel");
        assert!(identity.task_id.is_none());
    }

    #[test]
    fn parses_agent_identity_from_sub2api_credentials() {
        let raw = serde_json::json!({
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "auth_mode": "agentIdentity",
                "agent_runtime_id": "runtime-sub2api",
                "agent_private_key": agent_identity_private_key(),
                "task_id": "task-sub2api",
                "account_id": "team-sub2api",
                "chatgpt_account_id": "team-sub2api",
                "chatgpt_user_id": "user-sub2api",
                "email": "agent@example.com"
            }
        });

        let identity = parse_agent_identity_from_value(&raw)
            .expect("parse Sub2API Agent Identity")
            .expect("Agent Identity should be detected");

        assert_eq!(identity.agent_runtime_id, "runtime-sub2api");
        assert_eq!(identity.account_id, "team-sub2api");
        assert_eq!(identity.task_id.as_deref(), Some("task-sub2api"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recognized_web_session_imports_as_quota_only_token_account() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-web-session-quota-only-import-test");
        let access_token = make_jwt(serde_json::json!({
            "exp": chrono::Utc::now().timestamp() + 3600,
            "https://api.openai.com/profile": {
                "email": "quota-session@example.com"
            },
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-quota",
                "chatgpt_account_id": "account-quota",
                "chatgpt_plan_type": "plus"
            }
        }));
        let content = serde_json::json!({
            "user": {
                "id": "user-quota",
                "email": "quota-session@example.com",
                "name": "Quota Session"
            },
            "account": {
                "id": "account-quota",
                "planType": "plus",
                "structure": "personal"
            },
            "accessToken": access_token,
            "authProvider": "openai",
            "sessionToken": "must-not-become-agent-identity"
        });

        let accounts =
            import_from_json(&serde_json::to_string(&content).expect("serialize Web Session"))
                .await
                .expect("import Web Session");

        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert!(!account.is_agent_identity_auth());
        assert!(account.is_web_session_auth());
        assert_eq!(account.email, "quota-session@example.com");
        assert!(!account.tokens.access_token.is_empty());
        assert_ne!(
            account.tokens.access_token,
            "must-not-become-agent-identity"
        );
    }

    #[test]
    fn parses_sub2api_pkcs8_v1_agent_private_key_without_embedded_public_key() {
        let raw = serde_json::json!({
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "auth_mode": "agentIdentity",
                "agent_runtime_id": "runtime-sub2api-v1",
                "agent_private_key": sub2api_agent_identity_v1_private_key(),
                "account_id": "team-sub2api-v1",
                "chatgpt_account_id": "team-sub2api-v1",
                "chatgpt_user_id": "user-sub2api-v1",
                "plan_type": "k12"
            }
        });

        let identity = parse_agent_identity_from_value(&raw)
            .expect("parse Sub2API PKCS#8 v1 Agent Identity")
            .expect("Agent Identity should be detected");
        assert_eq!(identity.account_id, "team-sub2api-v1");
    }

    #[test]
    fn parses_sub2api_agent_identity_export_file_with_duplicate_account_fields() {
        let fixture = serde_json::json!({
            "type": "sub2api-data",
            "version": 1,
            "exported_at": "2026-07-21T14:58:07Z",
            "proxies": [],
            "accounts": [{
                "name": "fixture@example.com",
                "platform": "openai",
                "type": "oauth",
                "credentials": {
                    "account_id": "team-fixture",
                    "agent_private_key": agent_identity_private_key(),
                    "agent_runtime_id": "agent-fixture",
                    "auth_mode": "agentIdentity",
                    "chatgpt_account_id": "team-fixture",
                    "chatgpt_account_is_fedramp": false,
                    "chatgpt_user_id": "user-fixture",
                    "email": "fixture@example.com",
                    "id_token": "synthetic-id-token",
                    "plan_type": "k12",
                    "task_id": "task-fixture",
                    "workspace_id": "team-fixture"
                },
                "extra": {
                    "account_id": "team-fixture",
                    "chatgpt_account_id": "team-fixture",
                    "email": "fixture@example.com",
                    "source": "chatgpt_web_session",
                    "workspace_id": "team-fixture"
                },
                "concurrency": 10,
                "priority": 1,
                "rate_multiplier": 1,
                "auto_pause_on_expired": true
            }]
        });
        let path = std::env::temp_dir().join(format!(
            "cockpit-agent-identity-{}.json",
            uuid::Uuid::new_v4()
        ));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&fixture).expect("serialize fixture"),
        )
        .expect("write fixture");
        let content = fs::read_to_string(&path).expect("read fixture");
        let _ = fs::remove_file(&path);

        let values = super::codex_batch_import_values_from_content(&content)
            .expect("parse Sub2API export file");
        assert_eq!(values.len(), 1);
        let identity = parse_agent_identity_from_value(&values[0])
            .expect("parse Agent Identity")
            .expect("Agent Identity should be detected");

        assert_eq!(identity.account_id, "team-fixture");
        assert_eq!(identity.plan_type.as_deref(), Some("k12"));
        assert_eq!(identity.task_id.as_deref(), Some("task-fixture"));
    }

    #[test]
    fn agent_identity_storage_id_is_stable_per_chatgpt_account_member() {
        let build = |account_id: &str, user_id: &str, email: &str| {
            let identity = parse_agent_identity_from_value(&serde_json::json!({
                "auth_mode": "agentIdentity",
                "agent_runtime_id": format!("runtime-{email}"),
                "agent_private_key": agent_identity_private_key(),
                "account_id": account_id,
                "chatgpt_user_id": user_id,
                "email": email
            }))
            .expect("parse Agent Identity")
            .expect("Agent Identity should be detected");
            super::build_agent_identity_account_draft(identity)
                .expect("build Agent Identity account")
        };

        let first = build("team-a", "user-a", "first@example.com");
        let updated = build("team-a", "user-a", "updated@example.com");
        let other_member = build("team-a", "user-b", "second@example.com");
        let other_team = build("team-b", "user-a", "first@example.com");

        assert_eq!(first.id, updated.id);
        assert_ne!(first.id, other_member.id);
        assert_ne!(first.id, other_team.id);
    }

    #[test]
    fn agent_identity_members_in_the_same_workspace_coexist() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-agent-identity-members-test");
        let build = |user_id: &str, email: &str, runtime_id: &str| CodexAgentIdentity {
            agent_runtime_id: runtime_id.to_string(),
            agent_private_key: agent_identity_private_key(),
            task_id: Some(format!("task-{user_id}")),
            account_id: "shared-k12-workspace".to_string(),
            chatgpt_user_id: user_id.to_string(),
            email: Some(email.to_string()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: false,
        };

        let mut first =
            upsert_agent_identity_account(build("user-a", "first@example.com", "runtime-a"))
                .expect("import first workspace member");
        first.account_note = Some("keep this note".to_string());
        save_account(&first).expect("save first member note");
        let second =
            upsert_agent_identity_account(build("user-b", "second@example.com", "runtime-b"))
                .expect("import second workspace member");
        let updated_first = upsert_agent_identity_account(build(
            "user-a",
            "updated@example.com",
            "runtime-a-updated",
        ))
        .expect("reimport first workspace member");

        assert_ne!(first.id, second.id);
        assert_eq!(first.id, updated_first.id);
        assert_eq!(
            updated_first.account_note.as_deref(),
            Some("keep this note")
        );
        assert_eq!(
            updated_first
                .agent_identity
                .as_ref()
                .map(|identity| identity.agent_runtime_id.as_str()),
            Some("runtime-a-updated")
        );
        let index = load_account_index();
        assert_eq!(index.accounts.len(), 2);
        assert!(index.accounts.iter().any(|item| item.id == first.id));
        assert!(index.accounts.iter().any(|item| item.id == second.id));
    }

    #[test]
    fn agent_identity_legacy_storage_id_is_reused_only_for_matching_member() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-agent-identity-legacy-test");
        let identity = CodexAgentIdentity {
            agent_runtime_id: "runtime-new".to_string(),
            agent_private_key: agent_identity_private_key(),
            task_id: Some("task-new".to_string()),
            account_id: "legacy-k12-workspace".to_string(),
            chatgpt_user_id: "legacy-user".to_string(),
            email: Some("legacy@example.com".to_string()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: false,
        };
        let mut legacy = build_agent_identity_account_draft(identity.clone())
            .expect("build legacy Agent Identity account");
        legacy.id = build_legacy_agent_identity_account_id(&identity.account_id);
        legacy.account_note = Some("legacy note".to_string());
        save_account(&legacy).expect("save legacy account");
        save_account_index(&build_test_account_index(&legacy)).expect("save legacy index");

        let updated =
            upsert_agent_identity_account(identity.clone()).expect("reimport legacy account");

        assert_eq!(updated.id, legacy.id);
        assert_eq!(updated.account_note.as_deref(), Some("legacy note"));
        let mut other_member = identity;
        other_member.chatgpt_user_id = "other-user".to_string();
        other_member.email = Some("other@example.com".to_string());
        other_member.agent_runtime_id = "runtime-other".to_string();
        other_member.task_id = Some("task-other".to_string());
        let imported_other =
            upsert_agent_identity_account(other_member).expect("import other workspace member");

        assert_ne!(imported_other.id, legacy.id);
        assert_eq!(
            load_account(&legacy.id)
                .and_then(|account| account.agent_identity)
                .map(|identity| identity.chatgpt_user_id),
            Some("legacy-user".to_string())
        );
        let index = load_account_index();
        assert_eq!(index.accounts.len(), 2);
        assert_eq!(
            index.current_account_id.as_deref(),
            Some(legacy.id.as_str())
        );
    }

    #[test]
    fn agent_identity_prepare_is_rejected_as_api_service_only() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-agent-identity-prepare-test");
        let identity = parse_agent_identity_from_value(&serde_json::json!({
            "auth_mode": "agentIdentity",
            "agent_runtime_id": "runtime-prepare",
            "agent_private_key": agent_identity_private_key(),
            "account_id": "team-prepare",
            "chatgpt_user_id": "user-prepare"
        }))
        .expect("parse Agent Identity")
        .expect("Agent Identity should be detected");
        let account = super::build_agent_identity_account_draft(identity)
            .expect("build Agent Identity account");
        save_account(&account).expect("save Agent Identity account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(super::prepare_account_for_injection_from_auth_dir(
                &account.id,
                None,
            ))
            .expect_err("Agent Identity must remain API-service-only");

        assert!(error.contains("仅支持 API 服务"));
        let switch_error = runtime
            .block_on(super::switch_account_managed(&account.id))
            .expect_err("Agent Identity must not be switchable");
        assert!(switch_error.contains("仅支持 API 服务"));
    }

    #[test]
    fn agent_identity_cannot_be_used_as_api_key_oauth_binding() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-agent-identity-oauth-binding-test");
        let identity = parse_agent_identity_from_value(&serde_json::json!({
            "auth_mode": "agentIdentity",
            "agent_runtime_id": "runtime-binding",
            "agent_private_key": agent_identity_private_key(),
            "account_id": "team-binding",
            "chatgpt_user_id": "user-binding"
        }))
        .expect("parse Agent Identity")
        .expect("Agent Identity should be detected");
        let mut agent_account = super::build_agent_identity_account_draft(identity)
            .expect("build Agent Identity account");
        agent_account.tokens.refresh_token = Some("refresh-token".to_string());
        save_account(&agent_account).expect("save Agent Identity account");
        let api_key_account = CodexAccount::new_api_key(
            "api-binding".to_string(),
            "api-binding@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example.com/v1".to_string()),
            None,
            None,
            Vec::new(),
        );

        let error =
            super::validate_api_key_bound_oauth_account(&api_key_account, &agent_account.id)
                .expect_err("Agent Identity must not be accepted as an OAuth binding");

        assert!(error.contains("不能作为 OAuth 绑定账号"));
    }

    #[test]
    fn parse_line_delimited_json_values_accepts_one_object_per_line() {
        let raw = r#"{"id_token":"id-1","access_token":"access-1"}
{"id_token":"id-2","access_token":"access-2"}"#;

        let values = parse_line_delimited_json_values(raw)
            .expect("json lines should parse")
            .expect("multiple non-empty lines should return values");

        assert_eq!(values.len(), 2);
        assert_eq!(
            values[0].get("id_token").and_then(|value| value.as_str()),
            Some("id-1")
        );
        assert_eq!(
            values[1]
                .get("access_token")
                .and_then(|value| value.as_str()),
            Some("access-2")
        );
    }

    #[test]
    fn compat_parses_portable_codex_token_account() {
        let id_token = make_jwt(serde_json::json!({
            "email": "portable@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-portable",
                "chatgpt_plan_type": "plus",
                "account_id": "acc-portable"
            }
        }));
        let summary = CodexAccountSummary {
            id: "stored-portable".to_string(),
            email: "summary@example.com".to_string(),
            plan_type: None,
            subscription_active_until: None,
            created_at: 100,
            last_used: 200,
        };
        let account = parse_codex_account_compat(
            serde_json::json!({
                "id_token": id_token,
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "last_refresh": 300,
                "type": "codex"
            }),
            "stored-portable",
            Some(&summary),
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.id, "stored-portable");
        assert_eq!(account.email, "portable@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-portable"));
        assert_eq!(account.plan_type.as_deref(), Some("plus"));
        assert_eq!(account.account_id.as_deref(), Some("acc-portable"));
        assert_eq!(account.created_at, 100);
        assert_eq!(account.last_used, 200);
        assert_eq!(account.token_updated_at, Some(300));
    }

    #[test]
    fn compat_parses_portable_codex_api_key_account() {
        let account = parse_codex_account_compat(
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test-portable",
                "api_base_url": "https://example.com/v1",
                "api_provider_id": "custom-openai",
                "api_provider_name": "Custom OpenAI",
                "api_wire_api": "responses",
                "api_supports_websockets": true,
                "email": "api@example.com",
                "created_at": 100,
                "last_used": 200
            }),
            "stored-apikey",
            None,
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.id, "stored-apikey");
        assert!(account.is_api_key_auth());
        assert_eq!(account.email, "api@example.com");
        assert_eq!(account.openai_api_key.as_deref(), Some("sk-test-portable"));
        assert_eq!(
            account.api_base_url.as_deref(),
            Some("https://example.com/v1")
        );
        assert_eq!(account.api_provider_id.as_deref(), Some("custom-openai"));
        assert_eq!(account.api_provider_name.as_deref(), Some("Custom OpenAI"));
        assert_eq!(account.api_wire_api.as_deref(), Some("responses"));
        assert!(account.api_supports_websockets);
        assert_eq!(account.created_at, 100);
        assert_eq!(account.last_used, 200);
    }

    #[test]
    fn portable_api_key_import_projects_its_own_relay_credentials() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-portable-api-key-import-projection-test");
        let account = parse_codex_account_compat(
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-imported-relay",
                "api_base_url": "https://imported-relay.example.com/v1",
                "api_provider_id": "imported_relay",
                "api_provider_name": "Imported Relay",
                "api_wire_api": "responses",
                "api_supports_websockets": true,
                "email": "imported-relay@example.com"
            }),
            "portable-import-source",
            None,
        )
        .expect("parse portable API key account")
        .expect("portable API key account");

        let mut imported = super::import_account_struct(account).expect("import API key account");
        assert_eq!(imported.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(imported.api_provider_id.as_deref(), Some("imported_relay"));
        assert_eq!(
            imported.api_provider_name.as_deref(),
            Some("Imported Relay")
        );

        let profile_dir = env.home_dir.join("imported-relay-profile");
        write_account_bundle_to_dir(&profile_dir, &imported)
            .expect("project imported API key account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read imported auth"),
        )
        .expect("parse imported auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-imported-relay");
        let config =
            fs::read_to_string(profile_dir.join("config.toml")).expect("read imported config");
        assert!(config.contains("openai_base_url = \"https://imported-relay.example.com/v1\""));
        assert!(!config.contains("codex_local_access"));
        assert!(!config.contains("[model_providers.imported_relay]"));

        sync_api_key_account_from_local_state(&mut imported, &profile_dir);
        assert_eq!(imported.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(imported.api_provider_id.as_deref(), Some("imported_relay"));
        assert_eq!(
            imported.api_provider_name.as_deref(),
            Some("Imported Relay")
        );
    }

    #[test]
    fn compat_disables_websockets_for_chat_completions_account() {
        let account = parse_codex_account_compat(
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test-chat",
                "api_base_url": "https://example.com/v1",
                "api_wire_api": "chat_completions",
                "api_supports_websockets": true,
                "created_at": 100,
                "last_used": 200
            }),
            "stored-chat-apikey",
            None,
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.api_wire_api.as_deref(), Some("chat_completions"));
        assert!(!account.api_supports_websockets);
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }

    struct TestEnvGuard {
        home_dir: std::path::PathBuf,
        previous_home: Option<String>,
        previous_codex_home: Option<String>,
        previous_data_dir: Option<String>,
    }

    impl TestEnvGuard {
        fn new(prefix: &str) -> Self {
            let home_dir = make_temp_dir(prefix);
            let codex_home = home_dir.join(".codex");
            let test_data_dir = home_dir.join(".antigravity_cockpit");
            fs::create_dir_all(&codex_home).expect("create codex home");
            fs::create_dir_all(&test_data_dir).expect("create test data dir");

            let previous_home = std::env::var("HOME").ok();
            let previous_codex_home = std::env::var("CODEX_HOME").ok();
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR")
                .ok()
                .or_else(|| std::env::var("COCKPIT_TOOLS_DATA_DIR").ok());
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("CODEX_HOME", &codex_home);
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &test_data_dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &test_data_dir);

            Self {
                home_dir,
                previous_home,
                previous_codex_home,
                previous_data_dir,
            }
        }

        fn codex_home(&self) -> std::path::PathBuf {
            self.home_dir.join(".codex")
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match self.previous_home.as_ref() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match self.previous_codex_home.as_ref() {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
            match self.previous_data_dir.as_ref() {
                Some(value) => {
                    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value);
                    std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value);
                }
                None => {
                    std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
                    std::env::remove_var("COCKPIT_TOOLS_DATA_DIR");
                }
            }
            let _ = fs::remove_dir_all(&self.home_dir);
        }
    }

    #[test]
    fn test_env_guard_redirects_codex_account_storage() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-account-storage-isolation-test");

        let storage_path = get_accounts_storage_path();

        assert!(
            storage_path.starts_with(&env.home_dir),
            "Codex account storage should stay inside the test home, got {} for test home {}",
            storage_path.display(),
            env.home_dir.display()
        );
    }

    fn make_jwt(payload: serde_json::Value) -> String {
        let header = serde_json::json!({ "alg": "none", "typ": "JWT" });
        format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("serialize header")),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("serialize payload"))
        )
    }

    fn make_codex_tokens(
        email: &str,
        account_id: &str,
        organization_id: &str,
        suffix: &str,
        refresh_token: &str,
    ) -> CodexTokens {
        let id_token = make_jwt(serde_json::json!({
            "aud": ["codex-cli"],
            "iss": "https://auth.openai.com",
            "email": email,
            "sub": format!("user-{}", suffix),
            "exp": 4_102_444_800i64,
            "https://api.openai.com/auth": {
                "chatgpt_user_id": format!("user-{}", suffix),
                "chatgpt_plan_type": "pro",
                "account_id": account_id,
                "organization_id": organization_id,
            }
        }));
        let access_token = make_jwt(serde_json::json!({
            "sub": format!("access-{}", suffix),
            "exp": 4_102_444_800i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "organization_id": organization_id,
            }
        }));

        CodexTokens {
            id_token,
            access_token,
            refresh_token: Some(refresh_token.to_string()),
        }
    }

    fn build_test_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let email = "demo@example.com";
        let account_id = "acc-current";
        let organization_id = "org-current";
        let storage_id = build_account_storage_id(email, Some(account_id), Some(organization_id));

        let mut account = CodexAccount::new(storage_id.clone(), email.to_string(), tokens);
        account.user_id = Some("user-current".to_string());
        account.plan_type = Some("pro".to_string());
        account.account_id = Some(account_id.to_string());
        account.organization_id = Some(organization_id.to_string());
        account
    }

    #[test]
    fn clears_only_retired_app_server_preflight_reauth_state() {
        let mut affected = build_test_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "retired-app-server-preflight",
            "rt-retired-app-server-preflight",
        ));
        affected.requires_reauth = true;
        affected.reauth_reason = Some(
            "官方 app-server 返回 invalid_refresh_token，账号无法切换，请重新授权".to_string(),
        );

        assert!(clear_retired_app_server_preflight_reauth(&mut affected));
        assert!(!affected.requires_reauth);
        assert_eq!(affected.reauth_reason, None);

        let mut genuine = build_test_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "genuine-invalid-grant",
            "rt-genuine-invalid-grant",
        ));
        genuine.requires_reauth = true;
        genuine.reauth_reason = Some("refresh_token_invalidated: invalid_grant".to_string());

        assert!(!clear_retired_app_server_preflight_reauth(&mut genuine));
        assert!(genuine.requires_reauth);
        assert_eq!(
            genuine.reauth_reason.as_deref(),
            Some("refresh_token_invalidated: invalid_grant")
        );
    }

    #[test]
    fn reads_sub2api_codex_fingerprint_mode_from_extra() {
        let value = serde_json::json!({
            "extra": { "codex_fingerprint_mode": " FULL " }
        });
        assert_eq!(
            super::read_codex_fingerprint_mode(&value).as_deref(),
            Some("full")
        );
        assert_eq!(
            super::read_codex_fingerprint_mode(
                &serde_json::json!({"extra": {"codex_fingerprint_mode": "session"}})
            )
            .as_deref(),
            Some("session")
        );
        assert_eq!(
            super::resolved_codex_fingerprint_mode_value(None),
            "session"
        );
        assert_eq!(
            super::resolved_codex_fingerprint_mode_value(Some("SESSION")),
            "session"
        );
        assert_eq!(
            super::resolved_codex_fingerprint_mode_value(Some("off")),
            "off"
        );
    }

    fn seed_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let account = build_test_oauth_account(tokens);
        save_account(&account).expect("save account");

        let index = build_test_account_index(&account);
        save_account_index(&index).expect("save index");

        account
    }

    fn build_test_account_index(account: &CodexAccount) -> CodexAccountIndex {
        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
        index.current_account_id = Some(account.id.clone());
        index
    }

    fn write_test_account(data_dir: &Path, account: &CodexAccount) {
        let accounts_dir = data_dir.join("codex_accounts");
        fs::create_dir_all(&accounts_dir).expect("create test accounts dir");
        fs::write(
            accounts_dir.join(format!("{}.json", account.id)),
            serde_json::to_string_pretty(account).expect("serialize test account"),
        )
        .expect("write test account");
    }

    fn load_test_account(data_dir: &Path, account_id: &str) -> CodexAccount {
        let path = data_dir
            .join("codex_accounts")
            .join(format!("{}.json", account_id));
        let content = fs::read_to_string(&path).expect("read test account");
        serde_json::from_str(&content).expect("parse test account")
    }

    #[test]
    fn load_account_clears_bound_oauth_local_gateway_flag() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let _env = TestEnvGuard::new("codex-bound-oauth-clear-gateway");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-clear-gateway".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;
        save_account(&account).expect("save account");

        let loaded = load_account(&account.id).expect("load account");
        assert_eq!(loaded.bound_oauth_account_id.as_deref(), Some("oauth-1"));
        assert!(!loaded.bound_oauth_use_local_gateway);
    }

    #[test]
    fn load_account_keeps_bound_oauth_account_id_when_gateway_false() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let _env = TestEnvGuard::new("codex-bound-oauth-keep-id");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-keep-id".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = false;
        save_account(&account).expect("save account");

        let loaded = load_account(&account.id).expect("load account");
        assert_eq!(loaded.bound_oauth_account_id.as_deref(), Some("oauth-1"));
        assert!(!loaded.bound_oauth_use_local_gateway);
    }

    fn build_oauth_auth_file(tokens: &CodexTokens, account_id: &str) -> CodexAuthFile {
        CodexAuthFile {
            auth_mode: None,
            openai_api_key: Some(serde_json::Value::Null),
            base_url: None,
            tokens: Some(CodexAuthTokens {
                id_token: tokens.id_token.clone(),
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                account_id: Some(account_id.to_string()),
            }),
            agent_identity: None,
            personal_access_token: None,
            last_refresh: Some(serde_json::Value::String(
                "2026-04-13T00:00:00.000000Z".to_string(),
            )),
        }
    }

    fn write_oauth_auth_file(base_dir: &std::path::Path, tokens: &CodexTokens, account_id: &str) {
        let auth_file = build_oauth_auth_file(tokens, account_id);

        fs::create_dir_all(base_dir).expect("create auth dir");
        fs::write(
            base_dir.join("auth.json"),
            serde_json::to_string_pretty(&auth_file).expect("serialize auth file"),
        )
        .expect("write auth file");
    }

    #[test]
    fn build_auth_file_value_writes_empty_refresh_token_when_account_has_none() {
        let mut account = CodexAccount::new(
            "codex-cpa-account".to_string(),
            "cpa@example.com".to_string(),
            CodexTokens {
                id_token: "id.jwt.token".to_string(),
                access_token: "access.jwt.token".to_string(),
                refresh_token: None,
            },
        );
        account.account_id = Some("acc-cpa".to_string());

        let auth_file = build_auth_file_value(&account).expect("build auth file");
        let tokens = auth_file
            .get("tokens")
            .and_then(|value| value.as_object())
            .expect("tokens object");

        assert_eq!(
            tokens.get("refresh_token").and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            auth_file.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
    }

    #[test]
    fn build_auth_file_value_uses_real_token_update_time() {
        let mut account = CodexAccount::new(
            "codex-last-refresh".to_string(),
            "last-refresh@example.com".to_string(),
            CodexTokens {
                id_token: "id.jwt.token".to_string(),
                access_token: "access.jwt.token".to_string(),
                refresh_token: Some("rt_123".to_string()),
            },
        );
        account.account_id = Some("acc-last-refresh".to_string());
        account.token_updated_at = Some(1_700_000_000);

        let auth_file = build_auth_file_value(&account).expect("build auth file");
        assert_eq!(
            auth_file
                .get("last_refresh")
                .and_then(serde_json::Value::as_str),
            Some("2023-11-14T22:13:20.000000Z")
        );

        account.token_updated_at = None;
        let auth_file_without_refresh =
            build_auth_file_value(&account).expect("build auth file without refresh time");
        assert_eq!(
            auth_file_without_refresh.get("last_refresh"),
            Some(&serde_json::Value::Null)
        );
    }

    #[test]
    fn bundle_write_derives_workspace_id_from_coherent_token_pair() {
        let tokens = make_codex_tokens(
            "tuple@example.com",
            "acc-token",
            "org-token",
            "tuple",
            "rt-tuple",
        );
        let mut account = build_test_oauth_account(tokens);
        account.account_id = Some("acc-stale-metadata".to_string());
        account.organization_id = Some("org-stale-metadata".to_string());

        let resolved = super::resolve_account_for_bundle_write(Path::new("/tmp"), &account)
            .expect("resolve coherent credential tuple");

        assert_eq!(resolved.account_id.as_deref(), Some("acc-token"));
        assert_eq!(resolved.organization_id.as_deref(), Some("org-token"));
    }

    #[test]
    fn bundle_write_rejects_mixed_workspace_token_pair() {
        let mut tokens = make_codex_tokens(
            "tuple@example.com",
            "acc-id-token",
            "org-token",
            "tuple",
            "rt-tuple",
        );
        tokens.access_token = make_jwt(serde_json::json!({
            "sub": "access-other",
            "exp": 4_102_444_800i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access-token",
                "organization_id": "org-token"
            }
        }));
        let account = build_test_oauth_account(tokens);

        let error = super::resolve_account_for_bundle_write(Path::new("/tmp"), &account)
            .expect_err("mixed credential tuple must be rejected");

        assert!(error.contains("id_token_account_id=acc-id-token"));
        assert!(error.contains("access_token_account_id=acc-access-token"));
    }

    #[test]
    fn auth_credentials_store_mode_follows_codex_config() {
        let base_dir = make_temp_dir("codex-auth-store-mode-test");
        assert_eq!(
            super::codex_auth_credentials_store_mode(&base_dir),
            super::CodexAuthCredentialsStoreMode::File
        );

        for (raw_mode, expected) in [
            ("file", super::CodexAuthCredentialsStoreMode::File),
            ("keyring", super::CodexAuthCredentialsStoreMode::Keyring),
            ("auto", super::CodexAuthCredentialsStoreMode::Auto),
        ] {
            fs::write(
                base_dir.join("config.toml"),
                format!("cli_auth_credentials_store = \"{}\"\n", raw_mode),
            )
            .expect("write config");
            assert_eq!(
                super::codex_auth_credentials_store_mode(&base_dir),
                expected
            );
        }

        fs::remove_dir_all(base_dir).expect("remove temp dir");
    }

    #[test]
    fn account_switch_does_not_commit_when_runtime_stop_fails() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-switch-stop-failure-test");
        let account = CodexAccount::new_api_key(
            "api-switch-failure".to_string(),
            "api-switch-failure@example.com".to_string(),
            "sk-new".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
            Vec::new(),
        );
        save_account(&account).expect("save target account");
        let mut index = build_test_account_index(&account);
        index.current_account_id = None;
        save_account_index(&index).expect("save account index");

        let auth_path = env.codex_home().join("auth.json");
        let old_auth = "{\"sentinel\":\"old-auth\"}";
        fs::write(&auth_path, old_auth).expect("seed old auth");
        let observed_old_auth = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_in_hook = observed_old_auth.clone();
        let hook_auth_path = auth_path.clone();

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(super::switch_account_managed_with_before_commit(
                &account.id,
                move || async move {
                    observed_in_hook.store(
                        fs::read_to_string(hook_auth_path).expect("read auth in hook") == old_auth,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    Err("runtime stop failed".to_string())
                },
            ))
            .expect_err("switch must fail before commit");

        assert_eq!(error, "runtime stop failed");
        assert!(observed_old_auth.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(fs::read_to_string(auth_path).expect("read auth"), old_auth);
        assert!(load_account_index().current_account_id.is_none());
    }

    #[test]
    fn account_switch_does_not_stop_runtime_when_credential_prepare_fails() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-switch-prepare-failure-order-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "prepare-failure@example.com",
            "acc-prepare-failure",
            "org-prepare-failure",
            "prepare-failure",
            "rt-prepare-failure",
        ));
        account.requires_reauth = true;
        account.reauth_reason = Some("known refresh failure".to_string());
        account.tokens.access_token = make_jwt(serde_json::json!({
            "sub": "access-prepare-failure",
            "exp": 1i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-prepare-failure",
                "organization_id": "org-prepare-failure",
            }
        }));
        save_account(&account).expect("save target account");

        let stop_hook_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_in_hook = stop_hook_called.clone();
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(super::switch_account_managed_with_before_commit(
                &account.id,
                move || async move {
                    called_in_hook.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
            ))
            .expect_err("credential preparation must fail");

        assert_eq!(error, "known refresh failure", "unexpected error: {error}");
        assert!(!stop_hook_called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn token_refresh_file_lock_is_scoped_to_install_data_dir() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-shared-token-refresh-lock-test");

        let path = super::codex_token_refresh_file_lock_path("codex-account-id");

        assert!(path.starts_with(env.home_dir.join(".antigravity_cockpit/.cockpit-token-locks")));
        assert!(!path.to_string_lossy().contains("codex-account-id"));
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("lock")
        );
    }

    #[test]
    fn profile_mutation_lock_is_shared_by_installations_and_scoped_by_profile() {
        let first = std::path::PathBuf::from("/Users/tester/.codex");
        let second = std::path::PathBuf::from("/Users/tester/.codex");
        let other = std::path::PathBuf::from("/Users/tester/.codex-instance-2");

        assert_eq!(
            super::codex_profile_mutation_lock_path(&first),
            super::codex_profile_mutation_lock_path(&second)
        );
        assert_ne!(
            super::codex_profile_mutation_lock_path(&first),
            super::codex_profile_mutation_lock_path(&other)
        );
        assert!(super::codex_profile_mutation_lock_path(&first).starts_with(
            super::codex_profile_mutation_lock_root().join(".cockpit-profile-mutation-locks")
        ));
    }

    #[test]
    fn profile_mutation_lock_allows_one_writer_and_rejects_the_concurrent_writer() {
        let profile = std::env::temp_dir().join(format!(
            "cockpit-profile-mutation-lease-test-{}",
            std::process::id()
        ));
        let first = super::try_acquire_profile_mutation_lease(&profile, "test-first")
            .expect("first writer should acquire the profile lease");
        let second = match super::try_acquire_profile_mutation_lease(&profile, "test-second") {
            Ok(_) => panic!("concurrent writer must be rejected"),
            Err(error) => error,
        };
        assert!(second.contains("另一个 Cockpit Tools 环境正在操作"));

        drop(first);
        super::try_acquire_profile_mutation_lease(&profile, "test-after-release")
            .expect("profile lease should be reusable after release");
    }

    #[test]
    fn account_switch_commits_only_after_runtime_stop_hook() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-switch-stop-order-test");
        let account = CodexAccount::new_api_key(
            "api-switch-order".to_string(),
            "api-switch-order@example.com".to_string(),
            "sk-new".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
            Vec::new(),
        );
        save_account(&account).expect("save target account");
        let mut index = build_test_account_index(&account);
        index.current_account_id = None;
        save_account_index(&index).expect("save account index");

        let auth_path = env.codex_home().join("auth.json");
        let old_auth = "{\"sentinel\":\"old-auth\"}";
        fs::write(&auth_path, old_auth).expect("seed old auth");
        let observed_old_auth = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_in_hook = observed_old_auth.clone();
        let hook_auth_path = auth_path.clone();

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        runtime
            .block_on(super::switch_account_managed_with_before_commit(
                &account.id,
                move || async move {
                    observed_in_hook.store(
                        fs::read_to_string(hook_auth_path).expect("read auth in hook") == old_auth,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    Ok(())
                },
            ))
            .expect("switch account");

        assert!(observed_old_auth.load(std::sync::atomic::Ordering::SeqCst));
        let committed_auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(auth_path).expect("read committed auth"))
                .expect("parse committed auth");
        assert_eq!(
            committed_auth
                .get("auth_mode")
                .and_then(serde_json::Value::as_str),
            Some("apikey")
        );
        assert_eq!(
            committed_auth
                .get("OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str),
            Some("sk-new")
        );
        assert_eq!(
            load_account_index().current_account_id.as_deref(),
            Some(account.id.as_str())
        );
    }

    #[test]
    fn reauth_switch_preserves_new_tokens_and_marks_account_current() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-reauth-switch-preserves-new-token-test");
        let old_account = upsert_account(make_codex_tokens(
            "reauth-switch@example.com",
            "acc-reauth-switch",
            "org-reauth-switch",
            "old",
            "rt-old",
        ))
        .expect("seed old account");
        let mut index = build_test_account_index(&old_account);
        index.current_account_id = Some(old_account.id.clone());
        save_account_index(&index).expect("save old current account");

        let old_auth = build_auth_file_value(&old_account).expect("build old auth");
        fs::write(
            env.codex_home().join("auth.json"),
            serde_json::to_string_pretty(&old_auth).expect("serialize old auth"),
        )
        .expect("write old official auth");

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(
                "reauth-switch@example.com",
                "acc-reauth-switch",
                "org-reauth-switch",
                "new",
                "rt-new",
            ),
            &old_account.id,
        )
        .expect("save newly authorized tokens");
        assert_ne!(reauthed.tokens.id_token, old_account.tokens.id_token);

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let switched = runtime
            .block_on(
                super::switch_account_managed_after_reauth_with_before_commit_options(
                    &reauthed.id,
                    reauthed.token_generation,
                    || async { Ok(()) },
                ),
            )
            .expect("commit newly authorized tokens");

        assert_eq!(switched.tokens.id_token, reauthed.tokens.id_token);
        assert_eq!(switched.tokens.access_token, reauthed.tokens.access_token);
        assert_eq!(switched.tokens.refresh_token, reauthed.tokens.refresh_token);
        assert_eq!(
            load_account_index().current_account_id.as_deref(),
            Some(reauthed.id.as_str())
        );
        let persisted = load_account(&reauthed.id).expect("load switched account");
        assert_eq!(persisted.tokens.id_token, reauthed.tokens.id_token);
    }

    #[test]
    fn reauth_switch_rejects_changed_token_generation_before_stop() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-reauth-switch-generation-test");
        let account = seed_oauth_account(make_codex_tokens(
            "reauth-generation@example.com",
            "acc-reauth-generation",
            "org-reauth-generation",
            "new",
            "rt-new",
        ));
        let hook_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_in_hook = hook_called.clone();

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(
                super::switch_account_managed_after_reauth_with_before_commit(
                    &account.id,
                    account.token_generation.saturating_add(1),
                    move || async move {
                        called_in_hook.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    },
                ),
            )
            .expect_err("changed token generation must stop reauth switch");

        assert!(error.contains("凭据已发生变化"));
        assert!(!hook_called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn instance_launch_preflight_uses_local_credentials_without_internal_config_request() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-instance-launch-local-preflight-test");
        let account = seed_oauth_account(make_codex_tokens(
            "launch-local@example.com",
            "acc-launch-local",
            "org-launch-local",
            "launch-local",
            "rt-launch-local",
        ));

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(super::prepare_account_for_instance_launch_preflight(
                &account.id,
            ))
            .expect("instance launch preflight should use local credentials");

        assert_eq!(prepared.tokens.access_token, account.tokens.access_token);
        assert_eq!(prepared.token_generation, account.token_generation);
    }

    #[test]
    fn build_auth_file_value_marks_oauth_and_pat_as_codex_type() {
        let mut oauth = CodexAccount::new(
            "codex-oauth-type".to_string(),
            "oauth@type.example".to_string(),
            CodexTokens {
                id_token: "id.jwt.token".to_string(),
                access_token: "access.jwt.token".to_string(),
                refresh_token: Some("rt_123".to_string()),
            },
        );
        oauth.account_id = Some("acc-oauth".to_string());
        let oauth_file = build_auth_file_value(&oauth).expect("build oauth auth file");
        assert_eq!(
            oauth_file.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert!(oauth_file.get("personal_access_token").is_none());

        let pat = CodexAccount::new(
            "codex-pat-type".to_string(),
            "pat@type.example".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: "at-personal-token".to_string(),
                refresh_token: None,
            },
        );
        let pat_file = build_auth_file_value(&pat).expect("build pat auth file");
        assert_eq!(
            pat_file.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert_eq!(
            pat_file
                .get("personal_access_token")
                .and_then(serde_json::Value::as_str),
            Some("at-personal-token")
        );
        assert!(pat_file.get("tokens").is_none());

        let api_key = CodexAccount::new_api_key(
            "codex-api-type".to_string(),
            "api@type.example".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
            Vec::new(),
        );
        let api_file = build_auth_file_value(&api_key).expect("build api key auth file");
        assert!(api_file.get("type").is_none());
        assert_eq!(
            api_file
                .get("auth_mode")
                .and_then(serde_json::Value::as_str),
            Some("apikey")
        );
    }

    #[test]
    fn merge_existing_auth_file_keeps_extra_fields_and_strips_previous_faces() {
        let existing = serde_json::json!({
            "type": "codex",
            "email": "old@example.com",
            "OPENAI_API_KEY": "sk-old",
            "auth_mode": "apikey",
            "tokens": { "access_token": "old-token" },
            "personal_access_token": "at-old",
            "headers": { "User-Agent": "Custom" },
            "priority": 10
        })
        .as_object()
        .cloned();

        let mut account = CodexAccount::new(
            "codex-merge".to_string(),
            "next@example.com".to_string(),
            CodexTokens {
                id_token: "id.next.token".to_string(),
                access_token: "access.next.token".to_string(),
                refresh_token: Some("rt-next".to_string()),
            },
        );
        account.account_id = Some("acc-next".to_string());
        let next = build_auth_file_value(&account).expect("build next auth file");
        let merged = merge_existing_auth_file_value(existing, next);

        assert_eq!(
            merged.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert!(merged.get("email").is_none());
        assert!(merged.get("auth_mode").is_none());
        assert!(merged.get("personal_access_token").is_none());
        assert_eq!(merged.get("OPENAI_API_KEY"), Some(&serde_json::Value::Null));
        assert_eq!(
            merged
                .pointer("/tokens/access_token")
                .and_then(serde_json::Value::as_str),
            Some("access.next.token")
        );
        assert_eq!(
            merged
                .pointer("/headers/User-Agent")
                .and_then(serde_json::Value::as_str),
            Some("Custom")
        );
        assert_eq!(merged.get("priority"), Some(&serde_json::json!(10)));
    }

    #[test]
    fn write_auth_file_to_dir_merges_existing_official_fields() {
        let base_dir = make_temp_dir("codex-auth-merge-write-test");
        fs::write(
            base_dir.join("auth.json"),
            serde_json::json!({
                "type": "codex",
                "email": "old@example.com",
                "OPENAI_API_KEY": "sk-old",
                "custom_device_id": "keep-me"
            })
            .to_string(),
        )
        .expect("seed existing auth.json");

        let mut account = CodexAccount::new(
            "codex-merge-write".to_string(),
            "next@example.com".to_string(),
            CodexTokens {
                id_token: "id.next.token".to_string(),
                access_token: "access.next.token".to_string(),
                refresh_token: Some("rt-next".to_string()),
            },
        );
        account.account_id = Some("acc-next".to_string());
        write_auth_file_to_dir(&base_dir, &account).expect("write merged auth.json");

        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read merged auth.json"),
        )
        .expect("parse merged auth.json");
        assert_eq!(
            auth.get("custom_device_id")
                .and_then(serde_json::Value::as_str),
            Some("keep-me")
        );
        assert!(auth.get("email").is_none());
        assert_eq!(
            auth.get("type").and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert_eq!(auth.get("OPENAI_API_KEY"), Some(&serde_json::Value::Null));
        assert_eq!(
            auth.pointer("/tokens/access_token")
                .and_then(serde_json::Value::as_str),
            Some("access.next.token")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn extract_tokens_from_flat_codex_json() {
        let value = serde_json::json!({
            "id_token": "id.jwt.token",
            "access_token": "access.jwt.token",
            "refresh_token": "rt_123",
            "account_id": "acc_1",
            "type": "codex",
            "email": "demo@example.com"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_123"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_1"));
    }

    #[test]
    fn extract_tokens_from_flat_codex_json_does_not_use_session_token_as_refresh_token() {
        let value = serde_json::json!({
            "id_token": "id.jwt.token",
            "access_token": "access.jwt.token",
            "refresh_token": "",
            "session_token": "encrypted-session-token",
            "account_id": "acc_cpa",
            "type": "codex"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(account_id_hint.as_deref(), Some("acc_cpa"));
    }

    #[test]
    fn extract_tokens_from_nested_tokens_json() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "id.jwt.token",
                "access_token": "access.jwt.token",
                "refresh_token": "rt_456"
            },
            "account_id": "acc_2"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_456"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_2"));
    }

    #[test]
    fn extract_tokens_from_nested_tokens_json_does_not_use_session_token_as_refresh_token() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "id.jwt.token",
                "access_token": "access.jwt.token",
                "refresh_token": ""
            },
            "session_token": "encrypted-session-token",
            "account_id": "acc_nested"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(account_id_hint.as_deref(), Some("acc_nested"));
    }

    #[test]
    fn extract_tokens_from_camel_case_codex_json() {
        let value = serde_json::json!({
            "tokens": {
                "idToken": "id.jwt.token",
                "accessToken": "access.jwt.token",
                "refreshToken": "rt_789"
            },
            "accountId": "acc_3"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_789"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_3"));
    }

    #[test]
    fn extract_candidate_preserves_existing_token_priority() {
        let full_value = serde_json::json!({
            "idToken": "id.jwt.token",
            "accessToken": make_jwt(serde_json::json!({ "sub": "access-user" })),
            "refreshToken": "rt_existing"
        });
        let refresh_value = serde_json::json!({
            "refreshToken": "rt_existing",
            "accessToken": make_jwt(serde_json::json!({ "sub": "access-user" }))
        });
        let plain_token_value = serde_json::json!({
            "token": "not-a-jwt-token"
        });
        let opaque_access_token_value = serde_json::json!({
            "token": "at-confirmed-opaque-token",
            "email": "opaque@example.com",
            "account_id": "acc-opaque"
        });

        let full_candidate = extract_codex_import_candidate_from_value(&full_value)
            .expect("full token JSON should still be accepted");
        assert!(matches!(
            full_candidate,
            CodexJsonImportCandidate::FullToken { .. }
        ));

        let refresh_candidate = extract_codex_import_candidate_from_value(&refresh_value)
            .expect("refresh token should keep priority over accessToken-only");
        assert!(matches!(
            refresh_candidate,
            CodexJsonImportCandidate::RefreshToken { .. }
        ));

        assert!(
            extract_codex_import_candidate_from_value(&plain_token_value).is_none(),
            "plain token fields should not be treated as accessToken-only"
        );
        assert!(matches!(
            extract_codex_import_candidate_from_value(&opaque_access_token_value),
            Some(CodexJsonImportCandidate::AccessToken { .. })
        ));
    }

    #[test]
    fn extract_candidate_from_codex_session_json_as_cpa_tokens_without_session_token_refresh() {
        let access_token = make_jwt(serde_json::json!({
            "sub": "auth0|session-user",
            "https://api.openai.com/profile": {
                "email": "session@example.com",
                "email_verified": true
            },
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-session-token",
                "chatgpt_user_id": "user-session",
                "chatgpt_plan_type": "plus"
            }
        }));
        let session = serde_json::json!({
            "user": {
                "id": "user-session",
                "email": "session@example.com"
            },
            "expires": "2026-08-17T02:06:40.890Z",
            "account": {
                "id": "acc-session",
                "planType": "plus"
            },
            "accessToken": access_token,
            "authProvider": "openai",
            "sessionToken": "encrypted-session"
        });

        let candidate = extract_codex_import_candidate_from_value(&session)
            .expect("ChatGPT session JSON should be accepted");

        match candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                note_update,
                ..
            } => {
                assert_eq!(tokens.id_token, tokens.access_token);
                assert_eq!(tokens.refresh_token, None);
                assert_eq!(account_id_hint.as_deref(), Some("acc-session"));
                assert!(!super::has_codex_account_note_update(&note_update));
                assert!(decode_jwt_payload_value(&tokens.access_token).is_some());
            }
            _ => panic!("expected session JSON to be normalized to full CPA-style tokens"),
        }
    }

    #[test]
    fn extract_candidate_from_wrapped_codex_session_json_string() {
        let access_token = make_jwt(serde_json::json!({
            "email": "wrapped-session@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-wrapped-session"
            }
        }));
        let session = serde_json::json!({
            "user": {
                "email": "wrapped-session@example.com"
            },
            "account": {
                "id": "acc-wrapped-session"
            },
            "accessToken": access_token,
            "refreshToken": "rt_wrapped",
            "authProvider": "openai"
        });
        let wrapper = serde_json::json!({
            "session_json": serde_json::to_string(&session).expect("serialize session")
        });

        let candidate = extract_codex_import_candidate_from_value(&wrapper)
            .expect("wrapped session JSON string should be accepted");

        match candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                ..
            } => {
                assert_eq!(tokens.id_token, tokens.access_token);
                assert_eq!(tokens.refresh_token.as_deref(), Some("rt_wrapped"));
                assert_eq!(account_id_hint.as_deref(), Some("acc-wrapped-session"));
            }
            _ => panic!("expected wrapped session JSON to become full CPA-style tokens"),
        }
    }

    #[test]
    fn extract_candidate_from_sub2api_account_credentials() {
        let value = serde_json::json!({
            "name": "Sub2API account",
            "notes": "imported from sub2api",
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "sub2api@example.com",
                "access_token": "at-sub2api-team-token",
                "token_type": "Bearer",
                "auth_mode": "personal_access_token",
                "openai_auth_mode": "personal_access_token",
                "plan_type": "team",
                "chatgpt_account_id": "acc-sub2api",
                "expires_at": "2026-08-11T16:44:00Z",
                "subscription_expires_at": "2026-09-20T00:00:00Z"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("Sub2API account should expose access_token");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-sub2api-team-token");
                assert_eq!(hints.email.as_deref(), Some("sub2api@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-sub2api"));
                assert_eq!(
                    hints.subscription_active_until.as_deref(),
                    Some("2026-09-20T00:00:00Z")
                );
                assert_eq!(hints.account_note.as_deref(), Some("imported from sub2api"));
            }
            _ => panic!("expected accessToken-only candidate"),
        }
    }

    #[test]
    fn extract_candidate_does_not_treat_token_expiry_as_subscription_expiry() {
        let value = serde_json::json!({
            "name": "Sub2API access token",
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "token-expiry@example.com",
                "access_token": "at-token-expiry",
                "expires_at": "2026-08-11T16:44:00Z"
            },
            "expires_at": 1786466640,
            "auto_pause_on_expired": true
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("Sub2API access token should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken { hints, .. } => {
                assert_eq!(hints.subscription_active_until, None);
            }
            _ => panic!("expected accessToken-only candidate"),
        }
    }

    #[test]
    fn full_token_sub2api_candidate_preserves_explicit_subscription_expiry() {
        let id_token = make_jwt(serde_json::json!({
            "email": "oauth-expiry@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-oauth-expiry"
            }
        }));
        let access_token = make_jwt(serde_json::json!({
            "email": "oauth-expiry@example.com",
            "exp": 1_786_466_640,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-oauth-expiry",
                "chatgpt_plan_type": "plus",
                "chatgpt_subscription_active_until": "2090-01-01T00:00:00Z"
            }
        }));
        let value = serde_json::json!({
            "name": "Sub2API OAuth",
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "oauth-expiry@example.com",
                "id_token": id_token.clone(),
                "access_token": access_token.clone(),
                "refresh_token": "rt-oauth-expiry",
                "chatgpt_account_id": "acc-oauth-expiry",
                "expires_at": "2026-08-11T16:44:00Z",
                "subscription_expires_at": "2026-09-20T00:00:00Z"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("Sub2API OAuth account should be accepted");
        match &candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                subscription_active_until_hint,
                ..
            } => {
                assert_eq!(tokens.id_token, id_token);
                assert_eq!(tokens.access_token, access_token);
                assert_eq!(tokens.refresh_token.as_deref(), Some("rt-oauth-expiry"));
                assert_eq!(account_id_hint.as_deref(), Some("acc-oauth-expiry"));
                assert_eq!(
                    subscription_active_until_hint.as_deref(),
                    Some("2026-09-20T00:00:00Z")
                );

                let mut account = CodexAccount::new(
                    "codex-sub2api-oauth".to_string(),
                    "oauth-expiry@example.com".to_string(),
                    tokens.clone(),
                );
                account.account_id = account_id_hint.clone();
                let auth_file = build_auth_file_value(&account).expect("project auth.json");
                assert!(auth_file.get("personal_access_token").is_none());
                assert_eq!(
                    auth_file
                        .pointer("/tokens/refresh_token")
                        .and_then(serde_json::Value::as_str),
                    Some("rt-oauth-expiry")
                );
                assert_eq!(
                    auth_file
                        .pointer("/tokens/account_id")
                        .and_then(serde_json::Value::as_str),
                    Some("acc-oauth-expiry")
                );
            }
            _ => panic!("expected Sub2API OAuth credentials to become full tokens"),
        }

        let draft = super::codex_batch_import_draft_from_candidate(candidate);
        let preview = super::preview_account_for_draft(&draft)
            .expect("Sub2API OAuth preview should be available");

        assert_eq!(
            preview.subscription_active_until.as_deref(),
            Some("2026-09-20T00:00:00Z")
        );
    }

    #[test]
    fn extract_candidate_prefers_nested_full_oauth_over_opaque_access_token_fallback() {
        let id_token = make_jwt(serde_json::json!({
            "email": "opaque-oauth@example.com"
        }));
        let value = serde_json::json!({
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "idToken": id_token.clone(),
                "accessToken": "at-opaque-oauth-token",
                "refreshToken": "rt-opaque-oauth",
                "chatgptAccountId": "acc-opaque-oauth"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("nested OAuth credentials should be accepted");

        match candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                ..
            } => {
                assert_eq!(tokens.id_token, id_token);
                assert_eq!(tokens.access_token, "at-opaque-oauth-token");
                assert_eq!(tokens.refresh_token.as_deref(), Some("rt-opaque-oauth"));
                assert_eq!(account_id_hint.as_deref(), Some("acc-opaque-oauth"));
            }
            _ => panic!("expected nested credentials to remain full OAuth tokens"),
        }
    }

    #[test]
    fn extract_candidate_prefers_cpa_personal_access_token_over_session_token() {
        let session_id_token = make_jwt(serde_json::json!({
            "email": "cpa@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-cpa-session"
            }
        }));
        let session_access_token = make_jwt(serde_json::json!({
            "email": "cpa@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-cpa-session"
            }
        }));
        let value = serde_json::json!({
            "type": "codex",
            "provider": "openai",
            "id_token": session_id_token,
            "access_token": session_access_token,
            "refresh_token": "",
            "email": "cpa@example.com",
            "plan_type": "team",
            "account_id": "acc-cpa",
            "chatgpt_account_id": "acc-cpa-chatgpt",
            "at_token": "at-cpa-team-token",
            "personal_access_token": "at-cpa-personal-token",
            "token_type": "Bearer",
            "auth_mode": "personal_access_token",
            "openai_auth_mode": "personal_access_token",
            "headers": {
                "authorization": "Bearer at-cpa-header-token"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("CPA personal access token object should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-cpa-personal-token");
                assert_eq!(hints.email.as_deref(), Some("cpa@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-cpa"));
            }
            _ => panic!("expected CPA personal access token candidate"),
        }
    }

    #[test]
    fn extract_candidate_reads_workspace_id_from_custom_headers() {
        let value = serde_json::json!({
            "personal_access_token": "at-custom-header-token",
            "email": "workspace@example.com",
            "custom_headers": {
                "ChatGPT-Account-Id": "workspace-from-header"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("custom header workspace id should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-custom-header-token");
                assert_eq!(hints.account_id.as_deref(), Some("workspace-from-header"));
            }
            _ => panic!("expected access-token-only candidate"),
        }
    }

    #[test]
    fn extract_candidate_accepts_team_access_token_list_line() {
        let value = serde_json::Value::String(
            "team@example.comat-team-list-token.eyJhbGciOiJub25lIn0.payload".to_string(),
        );

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("team AT list line should expose the at-* token");

        match candidate {
            CodexJsonImportCandidate::AccessToken { access_token, .. } => {
                assert_eq!(access_token, "at-team-list-token");
            }
            _ => panic!("expected access-token-only candidate"),
        }
    }

    #[test]
    fn detects_sub2api_export_wrapper() {
        let value = serde_json::json!({
            "exported_at": "2026-05-18T09:40:35Z",
            "proxies": [],
            "accounts": [{
                "platform": "openai",
                "type": "oauth",
                "credentials": {
                    "access_token": make_jwt(serde_json::json!({ "sub": "sub2api-user" }))
                }
            }]
        });

        assert!(looks_like_sub2api_export(&value));
    }

    #[test]
    fn extract_candidate_accepts_opaque_access_token_with_hints() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "",
                "access_token": "at-confirmed-team-token",
                "refresh_token": ""
            },
            "email": "team@example.com",
            "plan_type": "team",
            "account_id": "acc-team",
            "organization_id": "org-team",
            "account_name": "Team Workspace",
            "account_structure": "team",
            "account_note": "confirmed import"
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("opaque at-* access token should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-confirmed-team-token");
                assert_eq!(hints.email.as_deref(), Some("team@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-team"));
                assert_eq!(hints.organization_id.as_deref(), Some("org-team"));
                assert_eq!(hints.account_name.as_deref(), Some("Team Workspace"));
                assert_eq!(hints.account_structure.as_deref(), Some("team"));
                assert_eq!(hints.account_note.as_deref(), Some("confirmed import"));
            }
            _ => panic!("expected opaque access-token-only candidate"),
        }
    }

    #[test]
    fn upsert_opaque_access_token_only_account_uses_import_hints() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-opaque-access-token-import-test");

        let account = upsert_account_from_access_token_with_hints(
            "at-confirmed-team-token".to_string(),
            CodexAccessTokenImportHints {
                email: Some("team@example.com".to_string()),
                user_id: Some("user-team".to_string()),
                plan_type: Some("team".to_string()),
                subscription_active_until: None,
                account_id: Some("acc-team".to_string()),
                organization_id: Some("org-team".to_string()),
                account_name: Some("Team Workspace".to_string()),
                account_structure: Some("team".to_string()),
                account_note: Some("confirmed import".to_string()),
                ..Default::default()
            },
        )
        .expect("upsert opaque access token account");

        assert_eq!(account.email, "team@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-team"));
        assert_eq!(account.plan_type.as_deref(), Some("team"));
        assert_eq!(account.account_id.as_deref(), Some("acc-team"));
        assert_eq!(account.organization_id.as_deref(), Some("org-team"));
        assert_eq!(account.account_name.as_deref(), Some("Team Workspace"));
        assert_eq!(account.account_structure.as_deref(), Some("team"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, "at-confirmed-team-token");
        assert_eq!(account.tokens.refresh_token, None);
        assert!(!account.requires_reauth);
        assert_eq!(account.reauth_reason, None);

        let persisted = load_account(&account.id).expect("persisted opaque account");
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
        assert_eq!(persisted.account_id.as_deref(), Some("acc-team"));
    }

    #[test]
    fn update_account_note_persists_personal_access_token_workspace_id() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-workspace-id-update-test");
        let account = upsert_account_from_access_token_with_hints(
            "at-workspace-update-token".to_string(),
            CodexAccessTokenImportHints {
                email: Some("workspace-update@example.com".to_string()),
                ..Default::default()
            },
        )
        .expect("create personal access token account");

        let updated = super::update_account_note(
            &account.id,
            super::CodexAccountNoteUpdate::default(),
            Some("  workspace-updated  ".to_string()),
        )
        .expect("update workspace id");

        assert_eq!(updated.account_id.as_deref(), Some("workspace-updated"));
        assert_eq!(
            load_account(&account.id)
                .expect("persisted account")
                .account_id
                .as_deref(),
            Some("workspace-updated")
        );
    }

    #[test]
    fn upsert_access_token_only_account_uses_access_claims() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-import-test");
        let access_token = make_jwt(serde_json::json!({
            "email": "access@example.com",
            "sub": "user-access",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access",
                "chatgpt_user_id": "user-access",
                "chatgpt_plan_type": "team",
                "chatgpt_subscription_active_until": 1767225600,
                "poid": "org-access"
            }
        }));

        let candidate = extract_codex_import_candidate_from_value(&serde_json::Value::String(
            access_token.clone(),
        ))
        .expect("raw JWT should be accepted as accessToken");
        assert!(matches!(
            candidate,
            CodexJsonImportCandidate::AccessToken { .. }
        ));

        let account = upsert_account_from_access_token(
            access_token.clone(),
            Some("imported from accessToken".to_string()),
        )
        .expect("upsert access token account");

        assert_eq!(account.email, "access@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-access"));
        assert_eq!(account.plan_type.as_deref(), Some("team"));
        assert_eq!(
            account.subscription_active_until.as_deref(),
            Some("1767225600")
        );
        assert_eq!(account.account_id.as_deref(), Some("acc-access"));
        assert_eq!(account.organization_id.as_deref(), Some("org-access"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token, None);
        assert_eq!(
            account.account_note.as_deref(),
            Some("imported from accessToken")
        );

        let persisted = load_account(&account.id).expect("persisted access token account");
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
    }

    #[test]
    fn upsert_auth_tokens_with_empty_id_token_uses_access_token() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-auth-file-access-token-import-test");
        let access_token = make_jwt(serde_json::json!({
            "email": "auth-access@example.com",
            "sub": "auth-access-user",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-auth-access",
                "chatgpt_user_id": "auth-access-user",
                "chatgpt_plan_type": "pro",
                "poid": "org-auth-access"
            }
        }));

        let account = upsert_account_from_auth_tokens(CodexAuthTokens {
            id_token: String::new(),
            access_token: access_token.clone(),
            refresh_token: None,
            account_id: None,
        })
        .expect("empty id_token auth tokens should import from accessToken");

        assert_eq!(account.email, "auth-access@example.com");
        assert_eq!(account.user_id.as_deref(), Some("auth-access-user"));
        assert_eq!(account.account_id.as_deref(), Some("acc-auth-access"));
        assert_eq!(account.organization_id.as_deref(), Some("org-auth-access"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token, None);
    }

    #[test]
    fn import_multiline_pending_oauth_array_creates_pending_account() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-pending-oauth-import-test");
        let content = r#"[
  {
    "id_token": "",
    "access_token": "",
    "refresh_token": "",
    "account_id": "",
    "last_refresh": "2026-07-04T02:25:18.829Z",
    "email": "dddd",
    "type": "codex",
    "expired": "",
    "account_note": "2131",
    "two_factor_secret": "Ddddd",
    "account_password": "213123",
    "phone_number": "2312",
    "mail_url": "https://mail.example.test/inbox?mail=dddd"
  }
]"#;
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(content))
            .expect("pending OAuth JSON array should import");

        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert_eq!(account.email, "dddd");
        assert!(is_pending_oauth_account(account));
        assert_eq!(
            account.authorization_status.as_deref(),
            Some(CODEX_AUTHORIZATION_STATUS_PENDING)
        );
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, "");
        assert_eq!(account.tokens.refresh_token, None);
        assert_eq!(account.account_note.as_deref(), Some("2131"));
        assert_eq!(account.two_factor_secret.as_deref(), Some("Ddddd"));
        assert_eq!(account.account_password.as_deref(), Some("213123"));
        assert_eq!(account.phone_number.as_deref(), Some("2312"));
        assert_eq!(
            account.mail_url.as_deref(),
            Some("https://mail.example.test/inbox?mail=dddd")
        );

        let persisted = load_account(&account.id).expect("pending account persisted");
        assert!(is_pending_oauth_account(&persisted));
        assert_eq!(persisted.account_note.as_deref(), Some("2131"));
        assert_eq!(
            persisted.mail_url.as_deref(),
            Some("https://mail.example.test/inbox?mail=dddd")
        );
    }

    #[test]
    fn import_pending_oauth_delimited_line_creates_pending_account() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-pending-oauth-delimited-import-test");
        let content = "user+tag@example.com----Pass@word123----BXU33BDMEBDIOAA2AOCFL4NBKVQAQWFY----https://mail.example.test/open.php?mail=user%2Btag%40example.com&pwd=secret&limit=5\nuser2@example.com----pwd2----ABCDEFGHIJKLMNOP";
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(content))
            .expect("delimited pending OAuth lines should import");

        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().all(is_pending_oauth_account));

        let first = accounts
            .iter()
            .find(|item| item.email == "user+tag@example.com")
            .expect("first account");
        assert_eq!(first.account_password.as_deref(), Some("Pass@word123"));
        assert_eq!(
            first.two_factor_secret.as_deref(),
            Some("BXU33BDMEBDIOAA2AOCFL4NBKVQAQWFY")
        );
        assert_eq!(
            first.mail_url.as_deref(),
            Some(
                "https://mail.example.test/open.php?mail=user%2Btag%40example.com&pwd=secret&limit=5"
            )
        );
        assert!(first.tokens.access_token.is_empty());

        let second = accounts
            .iter()
            .find(|item| item.email == "user2@example.com")
            .expect("second account");
        assert_eq!(second.account_password.as_deref(), Some("pwd2"));
        assert_eq!(
            second.two_factor_secret.as_deref(),
            Some("ABCDEFGHIJKLMNOP")
        );
        assert!(second.mail_url.is_none());
    }

    #[test]
    fn try_parse_pending_oauth_delimited_line_rejects_non_email() {
        assert!(try_parse_pending_oauth_delimited_line(
            "not-an-email----pwd----SECRET----https://example.com"
        )
        .is_none());
        assert!(try_parse_pending_oauth_delimited_line("rt_only_token").is_none());
        assert!(try_parse_pending_oauth_delimited_line(
            r#"{"email":"a@b.com","account_password":"x"}"#
        )
        .is_none());
    }

    #[test]
    fn import_auth_file_tokens_preserves_sensitive_note_metadata() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-auth-file-sensitive-note-import-test");
        let tokens = make_codex_tokens(
            "sensitive@example.com",
            "acc-sensitive",
            "org-sensitive",
            "sensitive",
            "rt-sensitive",
        );
        let content = serde_json::json!({
            "tokens": {
                "id_token": tokens.id_token,
                "access_token": tokens.access_token,
                "refresh_token": tokens.refresh_token,
                "account_id": "acc-sensitive"
            },
            "email": "sensitive@example.com",
            "type": "codex",
            "account_note": "note-1",
            "two_factor_secret": "SECRET-2FA",
            "account_password": "password-1",
            "phone_number": "15500000000",
            "mail_url": "https://mail.example.test/inbox"
        });
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(
                &serde_json::to_string(&content).expect("serialize import JSON"),
            ))
            .expect("auth file JSON should import");

        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert_eq!(account.email, "sensitive@example.com");
        assert_eq!(account.account_note.as_deref(), Some("note-1"));
        assert_eq!(account.two_factor_secret.as_deref(), Some("SECRET-2FA"));
        assert_eq!(account.account_password.as_deref(), Some("password-1"));
        assert_eq!(account.phone_number.as_deref(), Some("15500000000"));
        assert_eq!(
            account.mail_url.as_deref(),
            Some("https://mail.example.test/inbox")
        );

        let persisted = load_account(&account.id).expect("sensitive account persisted");
        assert_eq!(persisted.account_note.as_deref(), Some("note-1"));
        assert_eq!(persisted.two_factor_secret.as_deref(), Some("SECRET-2FA"));
        assert_eq!(persisted.account_password.as_deref(), Some("password-1"));
        assert_eq!(persisted.phone_number.as_deref(), Some("15500000000"));
        assert_eq!(
            persisted.mail_url.as_deref(),
            Some("https://mail.example.test/inbox")
        );
    }

    #[test]
    fn upsert_existing_account_keeps_own_refresh_token_when_import_has_none() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-preserve-refresh-token-test");
        let existing = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-existing",
        ));
        let mut imported_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "new",
            "rt-unused",
        );
        let imported_access_token = imported_tokens.access_token.clone();
        imported_tokens.refresh_token = None;

        let account = upsert_account(imported_tokens).expect("upsert existing account");

        assert_eq!(account.id, existing.id);
        assert_eq!(account.tokens.access_token, imported_access_token);
        assert_eq!(account.tokens.refresh_token.as_deref(), Some("rt-existing"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            Some("rt-existing")
        );
    }

    #[test]
    fn upsert_reuses_legacy_email_only_account_when_identity_appears() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-legacy-email-only-dedupe-test");
        let email = "legacy@example.com";
        let account_id = "acc-legacy";
        let organization_id = "org-legacy";
        let legacy_id = build_account_storage_id(email, None, None);
        let generated_identity_id =
            build_account_storage_id(email, Some(account_id), Some(organization_id));
        assert_ne!(legacy_id, generated_identity_id);

        let mut legacy = CodexAccount::new(
            legacy_id.clone(),
            email.to_string(),
            make_codex_tokens(email, account_id, organization_id, "old", "rt-existing"),
        );
        legacy.account_id = None;
        legacy.organization_id = None;
        save_account(&legacy).expect("save legacy account");

        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: legacy.id.clone(),
            email: legacy.email.clone(),
            plan_type: legacy.plan_type.clone(),
            subscription_active_until: legacy.subscription_active_until.clone(),
            created_at: legacy.created_at,
            last_used: legacy.last_used,
        });
        save_account_index(&index).expect("save legacy index");

        let imported = upsert_account(make_codex_tokens(
            email,
            account_id,
            organization_id,
            "new",
            "rt-new",
        ))
        .expect("upsert should reuse legacy account");

        assert_eq!(imported.id, legacy_id);
        assert_eq!(imported.account_id.as_deref(), Some(account_id));
        assert_eq!(imported.organization_id.as_deref(), Some(organization_id));
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, legacy_id);
        let index = load_account_index();
        assert_eq!(index.accounts.len(), 1);
        assert_eq!(index.accounts[0].id, legacy_id);
    }

    #[test]
    fn remove_accounts_prunes_missing_detail_index_entries() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-remove-prunes-missing-details-test");
        let account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-existing",
        ));
        let missing_id = "api-legacy-bound-oauth".to_string();
        let mut index = load_account_index();
        index.accounts.push(CodexAccountSummary {
            id: missing_id.clone(),
            email: "missing@example.com".to_string(),
            plan_type: Some("API_KEY".to_string()),
            subscription_active_until: None,
            created_at: 1,
            last_used: 1,
        });
        index.current_account_id = Some(missing_id.clone());
        save_account_index(&index).expect("save index with missing detail entry");

        let accounts = list_accounts_checked().expect("list should keep readable accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, account.id);

        remove_accounts(&[account.id.clone()]).expect("remove account");

        assert!(load_account(&account.id).is_none());
        let index = load_account_index();
        assert!(index.accounts.is_empty());
        assert!(index.current_account_id.is_none());
        let accounts = list_accounts_checked().expect("empty index should be valid");
        assert!(accounts.is_empty());
    }

    #[test]
    fn deleted_account_cannot_be_restored_by_stale_background_write() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-delete-tombstone-test");
        let account = upsert_account(make_codex_tokens(
            "deleted@example.com",
            "acc-deleted",
            "org-deleted",
            "old",
            "rt-old",
        ))
        .expect("seed account through normal authorization path");
        let stale_snapshot = account.clone();

        super::remove_account(&account.id).expect("remove account");
        let error = save_account(&stale_snapshot).expect_err("stale write must be rejected");
        assert!(error.contains("账号已删除或凭据快照已过期"));

        // 即使另一个旧进程绕过当前进程锁写回了详情文件，删除标记也必须让列表忽略它。
        super::save_account_unchecked(&stale_snapshot).expect("simulate stale external write");
        assert!(load_account(&account.id).is_none());
        assert!(list_accounts_checked().expect("list accounts").is_empty());

        let reauthorized = upsert_account(make_codex_tokens(
            "deleted@example.com",
            "acc-deleted",
            "org-deleted",
            "new",
            "rt-new",
        ))
        .expect("explicit authorization may recreate deleted account");
        assert_ne!(
            reauthorized.tokens.access_token,
            stale_snapshot.tokens.access_token
        );
        assert_eq!(reauthorized.id, stale_snapshot.id);
        assert!(reauthorized.token_generation > stale_snapshot.token_generation);

        let error = save_account(&stale_snapshot)
            .expect_err("old snapshot must remain rejected after reauthorization");
        assert!(error.contains("账号已删除或凭据快照已过期"));
        let loaded = load_account(&reauthorized.id).expect("load reauthorized account");
        assert_eq!(loaded.tokens.access_token, reauthorized.tokens.access_token);

        // 旧进程即使绕过新版本保护，在重新授权后覆盖详情文件，也不能再让旧 Token 被加载。
        super::save_account_unchecked(&stale_snapshot)
            .expect("simulate stale external write after reauthorization");
        let stale_load = super::load_account_with_summary(&reauthorized.id, None);
        assert!(
            stale_load.is_err(),
            "stale load should fail: result={:?}, tombstone={:?}",
            stale_load
                .as_ref()
                .ok()
                .and_then(|account| account.as_ref())
                .map(|account| account.token_generation),
            super::read_account_tombstone(&reauthorized.id),
        );
        let error = list_accounts_checked()
            .expect_err("stale external credentials must not be listed after reauthorization");
        assert!(error.contains("凭据快照已过期"));
    }

    #[test]
    fn list_accounts_prunes_orphan_index_when_all_details_are_missing() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-list-prunes-orphan-index-test");
        let missing_id = "api-legacy-bound-oauth".to_string();
        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: missing_id.clone(),
            email: "missing@example.com".to_string(),
            plan_type: Some("API_KEY".to_string()),
            subscription_active_until: None,
            created_at: 1,
            last_used: 1,
        });
        index.current_account_id = Some(missing_id);
        save_account_index(&index).expect("save orphan index");

        let accounts = list_accounts_checked().expect("orphan index should be pruned");
        assert!(accounts.is_empty());

        let index = load_account_index();
        assert!(index.accounts.is_empty());
        assert!(index.current_account_id.is_none());
    }

    #[test]
    fn list_accounts_recovers_details_missing_from_index_and_merges_summary_fields() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-list-recovers-missing-index-details-test");
        let mut indexed = build_test_oauth_account(make_codex_tokens(
            "indexed@example.com",
            "acc-indexed",
            "org-indexed",
            "indexed",
            "rt-indexed",
        ));
        indexed.id = build_account_storage_id(
            "indexed@example.com",
            Some("acc-indexed"),
            Some("org-indexed"),
        );
        indexed.email = "indexed@example.com".to_string();
        indexed.plan_type = None;
        indexed.subscription_active_until = None;
        indexed.created_at = 10;
        indexed.last_used = 10;
        save_account(&indexed).expect("save indexed detail");

        let mut hidden = build_test_oauth_account(make_codex_tokens(
            "hidden@example.com",
            "acc-hidden",
            "org-hidden",
            "hidden",
            "rt-hidden",
        ));
        hidden.id =
            build_account_storage_id("hidden@example.com", Some("acc-hidden"), Some("org-hidden"));
        hidden.email = "hidden@example.com".to_string();
        hidden.created_at = 20;
        hidden.last_used = 20;
        save_account(&hidden).expect("save hidden detail");

        let old_index = serde_json::json!({
            "version": "1.0",
            "accounts": [{
                "id": indexed.id,
                "email": indexed.email,
                "plan_type": "team",
                "subscription_active_until": "2026-08-01T00:00:00Z",
                "created_at": 5,
                "last_used": 30
            }],
            "current_account_id": indexed.id
        });
        fs::write(
            get_accounts_storage_path(),
            serde_json::to_string_pretty(&old_index).expect("serialize old index"),
        )
        .expect("write old index");

        let accounts = list_accounts_checked().expect("list should repair from details");
        assert_eq!(accounts.len(), 2);
        let listed_indexed = accounts
            .iter()
            .find(|account| account.id == indexed.id)
            .expect("indexed account should remain visible");
        assert_eq!(listed_indexed.plan_type.as_deref(), Some("team"));
        assert_eq!(
            listed_indexed.subscription_active_until.as_deref(),
            Some("2026-08-01T00:00:00Z")
        );
        assert!(accounts.iter().any(|account| account.id == hidden.id));

        let repaired_index = load_account_index();
        assert_eq!(
            repaired_index.detail_schema_version,
            CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION
        );
        assert_eq!(repaired_index.accounts.len(), 2);
        assert_eq!(
            repaired_index.current_account_id.as_deref(),
            Some(indexed.id.as_str())
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let repaired_detail = loop {
            let account = load_account(&indexed.id).expect("indexed detail should remain");
            if account.plan_type.as_deref() == Some("team") {
                break account;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background summary migration should persist"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(repaired_detail.plan_type.as_deref(), Some("team"));
        assert_eq!(
            repaired_detail.subscription_active_until.as_deref(),
            Some("2026-08-01T00:00:00Z")
        );
        assert_eq!(repaired_detail.created_at, 10);
        assert_eq!(repaired_detail.last_used, 30);
    }

    #[test]
    fn reauth_updates_explicit_target_account_even_when_identity_changes() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-explicit-reauth-target-test");
        let email = "reauth@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let generated_new_id = build_account_storage_id(email, Some("acc-new"), Some("org-new"));
        assert_ne!(existing.id, generated_new_id);

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update target account");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.account_id.as_deref(), Some("acc-new"));
        assert_eq!(reauthed.organization_id.as_deref(), Some("org-new"));
        assert_eq!(reauthed.tokens.refresh_token.as_deref(), Some("rt-new"));
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn reauth_preserves_note_details_when_target_is_missing_from_index() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-reauth-preserve-notes-missing-index-test");
        let email = "reauth-notes@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let mut detail = load_account(&existing.id).expect("load existing account");
        detail.account_name = Some("备注账号".to_string());
        detail.account_structure = Some("个人".to_string());
        detail.account_note = Some("其他备注".to_string());
        detail.two_factor_secret = Some("JBSWY3DPEHPK3PXP".to_string());
        detail.account_password = Some("password-1".to_string());
        detail.phone_number = Some("13800000000".to_string());
        save_account(&detail).expect("save noted account");

        let mut broken_index = CodexAccountIndex::new();
        broken_index.accounts.clear();
        broken_index.current_account_id = None;
        save_account_index(&broken_index).expect("save broken index");

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update detail-backed target");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.account_id.as_deref(), Some("acc-new"));
        assert_eq!(reauthed.organization_id.as_deref(), Some("org-new"));
        assert_eq!(reauthed.account_name.as_deref(), Some("备注账号"));
        assert_eq!(reauthed.account_structure.as_deref(), Some("个人"));
        assert_eq!(reauthed.account_note.as_deref(), Some("其他备注"));
        assert_eq!(
            reauthed.two_factor_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );
        assert_eq!(reauthed.account_password.as_deref(), Some("password-1"));
        assert_eq!(reauthed.phone_number.as_deref(), Some("13800000000"));

        let persisted = load_account(&existing.id).expect("load persisted account");
        assert_eq!(persisted.account_note.as_deref(), Some("其他备注"));
        assert_eq!(
            persisted.two_factor_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );

        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn reauth_removes_generated_duplicate_for_target_identity() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-explicit-reauth-dedupe-test");
        let email = "reauth-duplicate@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let duplicate = upsert_account(make_codex_tokens(
            email, "acc-new", "org-new", "dup", "rt-dup",
        ))
        .expect("seed duplicate account");
        assert_ne!(existing.id, duplicate.id);
        assert_eq!(list_accounts_checked().expect("list accounts").len(), 2);

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update target and remove duplicate");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.tokens.refresh_token.as_deref(), Some("rt-new"));
        assert!(load_account(&duplicate.id).is_none());
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn upsert_access_token_only_existing_account_keeps_own_refresh_token() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-preserve-refresh-test");
        let existing = upsert_account(make_codex_tokens(
            "access@example.com",
            "acc-access",
            "org-access",
            "old",
            "rt-existing",
        ))
        .expect("seed existing account");
        let access_token = make_jwt(serde_json::json!({
            "email": "access@example.com",
            "sub": "user-access-new",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access",
                "chatgpt_user_id": "user-access-new",
                "chatgpt_plan_type": "team",
                "poid": "org-access"
            }
        }));

        let account =
            upsert_account_from_access_token(access_token.clone(), None).expect("upsert AT only");

        assert_eq!(account.id, existing.id);
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token.as_deref(), Some("rt-existing"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            Some("rt-existing")
        );
    }

    #[test]
    fn extracts_email_from_openai_profile_claim() {
        let id_token = make_jwt(serde_json::json!({
            "aud": ["https://api.openai.com/v1"],
            "iss": "https://auth.openai.com",
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-profile",
                "chatgpt_plan_type": "plus",
                "account_id": "acc-profile"
            },
            "https://api.openai.com/profile": {
                "email": "profile@example.com",
                "email_verified": true
            }
        }));

        let (email, user_id, plan_type, _, account_id, _) =
            extract_user_info(&id_token).expect("extract profile email");

        assert_eq!(email, "profile@example.com");
        assert_eq!(user_id.as_deref(), Some("user-profile"));
        assert_eq!(plan_type.as_deref(), Some("plus"));
        assert_eq!(account_id.as_deref(), Some("acc-profile"));
    }

    #[test]
    fn parses_auth_file_last_refresh_variants() {
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!("2026-04-13T00:00:00.000000Z"))),
            Some(1_776_038_400)
        );
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!(1_765_497_600_123i64))),
            Some(1_765_497_600)
        );
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!(1_765_497_600i64))),
            Some(1_765_497_600)
        );
    }

    #[test]
    fn formats_refresh_errors_with_actionable_reason() {
        let reused = format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        );
        assert!(reused.contains("refresh_token 已被其它客户端或实例使用过"));
        assert!(reused.contains("请重新登录"));

        let unauthorized =
            format_refresh_error_for_user("Token 刷新失败: status=401 Unauthorized, body_len=42");
        assert!(unauthorized.contains("登录授权无效"));
        assert!(unauthorized.contains("请重新登录"));

        let region = format_refresh_error_for_user(
            "Token 刷新失败: status=403 Forbidden, error_code=unsupported_country_region_territory",
        );
        assert!(region.contains("当前网络地区不支持刷新 Codex 授权"));
        assert!(!region.contains("请重新登录"));
    }

    #[test]
    fn quota_refresh_ownership_errors_are_internal_only() {
        assert!(super::is_refresh_ownership_deferred_error(
            "官方 ChatGPT/Codex 客户端正在使用此账号；为避免重复轮换 refresh_token，Cockpit Tools 已暂停该账号刷新。"
        ));
        assert!(super::is_refresh_ownership_deferred_error(
            "该账号正在执行 Codex 实例启动或受控转移；为避免重复轮换 refresh_token，本次刷新已取消。"
        ));
        assert!(!super::is_refresh_ownership_deferred_error(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused"
        ));
        assert!(!super::is_refresh_ownership_deferred_error(
            "Codex 上游网络或代理不可用"
        ));
    }

    #[test]
    fn remote_api_auth_rejection_overrides_unexpired_access_token() {
        let mut account = CodexAccount::new(
            "codex-remote-auth-rejected".to_string(),
            "remote-auth-rejected@example.com".to_string(),
            make_codex_tokens(
                "remote-auth-rejected@example.com",
                "acc-remote-auth-rejected",
                "org-remote-auth-rejected",
                "remote-auth-rejected",
                "rt-remote-auth-rejected",
            ),
        );
        account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
            code: None,
            message: "API 返回错误 401 Unauthorized [body_len:22]".to_string(),
            timestamp: now_timestamp(),
        });
        assert!(super::account_has_remote_api_auth_rejection(&account));

        account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
            code: Some("refresh_token_reused".to_string()),
            message: "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused"
                .to_string(),
            timestamp: now_timestamp(),
        });
        assert!(!super::account_has_remote_api_auth_rejection(&account));
    }

    #[test]
    fn switch_auth_error_does_not_wrap_refresh_token_reused() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-switch-auth-error-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "api-only@example.com",
            "acc-api-only",
            "org-api-only",
            "api-only",
            "rt-api-only",
        ));
        account.requires_reauth = true;
        account.reauth_reason = Some(format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        ));
        save_account(&account).expect("save reauth account");

        assert_eq!(
            format_account_switch_error(&account.id, "fallback".to_string()),
            "fallback"
        );
    }

    #[test]
    fn switch_auth_error_does_not_claim_api_only_when_api_was_rejected() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-switch-auth-api-rejected-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "api-rejected@example.com",
            "acc-api-rejected",
            "org-api-rejected",
            "api-rejected",
            "rt-api-rejected",
        ));
        account.requires_reauth = true;
        account.reauth_reason = Some(
            "Codex 登录授权已被服务端撤销，无法自动刷新。请重新登录 Codex 账号。".to_string(),
        );
        account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
            code: None,
            message: "API 返回错误 401 Unauthorized [body_len:22]".to_string(),
            timestamp: now_timestamp(),
        });
        save_account(&account).expect("save rejected account");

        let encoded = format_account_switch_error(&account.id, "fallback".to_string());
        let payload = encoded
            .strip_prefix("CODEX_SWITCH_AUTH_REQUIRED:")
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
            .expect("structured switch auth failure");

        assert_eq!(payload["apiOnlyAvailable"], false);
    }

    #[test]
    fn switch_auth_error_treats_server_revocation_as_terminal_without_quota_error() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-switch-auth-server-revoked-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "server-revoked@example.com",
            "acc-server-revoked",
            "org-server-revoked",
            "server-revoked",
            "rt-server-revoked",
        ));
        account.requires_reauth = true;
        account.reauth_reason = Some(
            "Codex 登录授权已被服务端撤销，无法自动刷新。请重新登录 Codex 账号。"
                .to_string(),
        );
        save_account(&account).expect("save server-revoked account");

        let encoded = format_account_switch_error(&account.id, "fallback".to_string());
        let payload = encoded
            .strip_prefix("CODEX_SWITCH_AUTH_REQUIRED:")
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
            .expect("structured switch auth failure");

        assert_eq!(payload["reasonCode"], "refresh_token_invalidated");
        assert_eq!(payload["apiOnlyAvailable"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn clearing_client_auth_observation_preserves_credentials_and_reauth_state() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-skip-auth-check-clears-observation-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "skip-observation@example.com",
            "acc-skip-observation",
            "org-skip-observation",
            "skip-observation",
            "rt-skip-observation",
        ));
        account.client_auth_status = Some("login_required".to_string());
        account.last_client_auth_observed_at = Some(100);
        account.last_client_login_redirect_at = Some(101);
        account.last_client_launch_at = Some(99);
        account.last_client_auth_instance_id = Some("default".to_string());
        account.requires_reauth = true;
        account.reauth_reason = Some("真实 Token Authority 授权异常".to_string());
        save_account(&account).expect("save observed account");

        assert!(
            clear_client_auth_observation(&account.id)
                .await
                .expect("clear client observation")
        );
        let persisted = load_account(&account.id).expect("load cleared account");
        assert_eq!(persisted.client_auth_status, None);
        assert_eq!(persisted.last_client_auth_observed_at, None);
        assert_eq!(persisted.last_client_login_redirect_at, None);
        assert_eq!(persisted.last_client_launch_at, None);
        assert_eq!(persisted.last_client_auth_instance_id, None);
        assert!(persisted.requires_reauth);
        assert_eq!(
            persisted.reauth_reason.as_deref(),
            Some("真实 Token Authority 授权异常")
        );
    }

    #[test]
    fn client_auth_observation_does_not_wrap_switch_errors() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-client-auth-observation-no-switch-block-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "client-observation@example.com",
            "acc-client-observation",
            "org-client-observation",
            "client-observation",
            "rt-client-observation",
        ));
        account.client_auth_status = Some("login_required".to_string());
        save_account(&account).expect("save observed account");

        assert_eq!(
            format_account_switch_error(&account.id, "ordinary switch error".to_string()),
            "ordinary switch error"
        );
    }

    #[test]
    fn access_token_only_accounts_do_not_require_proactive_refresh() {
        let mut account = CodexAccount::new(
            "codex_access_only".to_string(),
            "access-only@example.com".to_string(),
            make_codex_tokens(
                "access-only@example.com",
                "acc-access-only",
                "org-access-only",
                "access-only",
                "rt-unused",
            ),
        );
        account.tokens.refresh_token = None;
        account.token_updated_at = Some(0);

        assert!(!is_managed_auth_refresh_due(&account));
    }

    #[test]
    fn explicit_instance_launch_does_not_refresh_a_valid_access_token() {
        let mut account = CodexAccount::new(
            "codex_launch_revalidate".to_string(),
            "launch-revalidate@example.com".to_string(),
            make_codex_tokens(
                "launch-revalidate@example.com",
                "acc-launch-revalidate",
                "org-launch-revalidate",
                "launch-revalidate",
                "rt-launch-revalidate",
            ),
        );
        account.requires_reauth = true;

        assert!(!managed_account_runtime_tokens_need_refresh(&account));
        assert!(!super::managed_account_refresh_needed_for_request(
            &account, true, true,
        ));
    }

    #[test]
    fn expired_id_token_does_not_trigger_runtime_refresh() {
        let mut account = CodexAccount::new(
            "codex_expired_id_token".to_string(),
            "expired-id@example.com".to_string(),
            make_codex_tokens(
                "expired-id@example.com",
                "acc-expired-id",
                "org-expired-id",
                "expired-id",
                "rt-expired-id",
            ),
        );
        account.tokens.id_token = make_jwt(serde_json::json!({ "exp": 1i64 }));
        account.token_updated_at = Some(now_timestamp());

        assert!(!is_managed_auth_refresh_due(&account));
        assert!(!super::managed_account_tokens_need_refresh(&account));
        assert!(!managed_account_runtime_tokens_need_refresh(&account));
    }

    #[test]
    fn client_runtime_allows_expired_id_token_when_access_token_is_valid() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-runtime-expired-id-token-result-test");
        let mut tokens = make_codex_tokens(
            "runtime-refresh-result@example.com",
            "acc-runtime-refresh-result",
            "org-runtime-refresh-result",
            "runtime-refresh-result",
            "rt-runtime-refresh-result",
        );
        tokens.id_token = make_jwt(serde_json::json!({
            "exp": 1i64,
            "email": "runtime-refresh-result@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-runtime-refresh-result",
                "chatgpt_user_id": "user-runtime-refresh-result",
                "chatgpt_plan_type": "plus",
                "poid": "org-runtime-refresh-result"
            }
        }));
        let account = seed_oauth_account(tokens);

        let prepared = super::finish_managed_runtime_account_refresh(account.clone(), true)
            .expect("expired id_token must not block client launch");
        assert_eq!(prepared.id, account.id);
        let persisted = load_account(&account.id).expect("load reauth account");
        assert!(!persisted.requires_reauth);
    }

    #[test]
    fn official_account_check_accepts_target_account_key() {
        let account = CodexAccount::new(
            "codex_account_check".to_string(),
            "check@example.com".to_string(),
            make_codex_tokens(
                "check@example.com",
                "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
                "org-check",
                "check",
                "rt-check",
            ),
        );
        let payload = serde_json::json!({
            "accounts": {
                "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3": {
                    "account": {
                        "account_id": "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
                        "account_residency_region": "no_constraint"
                    },
                    "can_access_with_session": true
                }
            },
            "account_ordering": ["3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3"]
        });

        super::validate_account_check_payload(&payload, &account)
            .expect("target account should pass official account check validation");
    }

    #[test]
    fn official_account_check_rejects_another_account() {
        let account = CodexAccount::new(
            "codex_account_check_mismatch".to_string(),
            "check-mismatch@example.com".to_string(),
            make_codex_tokens(
                "check-mismatch@example.com",
                "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
                "org-check",
                "check-mismatch",
                "rt-check-mismatch",
            ),
        );
        let payload = serde_json::json!({
            "accounts": {
                "6a7dc3f2-ea90-4456-9426-a46bd8b3e6f9": {
                    "name": "Another"
                }
            },
            "account_ordering": ["6a7dc3f2-ea90-4456-9426-a46bd8b3e6f9"]
        });

        let error = super::validate_account_check_payload(&payload, &account)
            .expect_err("another account must not pass target account validation");
        assert!(error.message.contains("与目标账号不一致"));
    }

    #[test]
    fn official_account_check_rejects_session_without_account_access() {
        let account = CodexAccount::new(
            "codex_account_check_denied".to_string(),
            "check-denied@example.com".to_string(),
            make_codex_tokens(
                "check-denied@example.com",
                "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
                "org-check",
                "check-denied",
                "rt-check-denied",
            ),
        );
        let payload = serde_json::json!({
            "accounts": {
                "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3": {
                    "account": {
                        "account_id": "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3"
                    },
                    "can_access_with_session": false
                }
            }
        });

        let error = super::validate_account_check_payload(&payload, &account)
            .expect_err("session without account access must be rejected");
        assert!(error.message.contains("不允许当前登录态访问目标账号"));
    }

    #[test]
    fn id_token_within_refresh_lead_does_not_require_runtime_refresh() {
        let mut account = CodexAccount::new(
            "codex_id_token_refresh_lead".to_string(),
            "id-token-lead@example.com".to_string(),
            make_codex_tokens(
                "id-token-lead@example.com",
                "acc-id-token-lead",
                "org-id-token-lead",
                "id-token-lead",
                "rt-id-token-lead",
            ),
        );
        account.tokens.id_token = make_jwt(serde_json::json!({
            "exp": now_timestamp() + crate::modules::codex_oauth::ID_TOKEN_REFRESH_LEAD_SECONDS - 30,
        }));
        account.token_updated_at = Some(now_timestamp());

        assert!(!is_managed_auth_refresh_due(&account));
        assert!(!managed_account_runtime_tokens_need_refresh(&account));
    }

    #[test]
    fn runtime_prepare_projects_expired_id_token_when_access_token_is_valid() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-runtime-expired-id-token-test");
        let mut tokens = make_codex_tokens(
            "runtime-expired@example.com",
            "acc-runtime-expired",
            "org-runtime-expired",
            "runtime-expired",
            "rt-unused",
        );
        tokens.id_token = make_jwt(serde_json::json!({
            "exp": 1i64,
            "email": "runtime-expired@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-runtime-expired",
                "chatgpt_user_id": "user-runtime-expired",
                "chatgpt_plan_type": "plus",
                "poid": "org-runtime-expired"
            }
        }));
        tokens.refresh_token = None;
        let account = seed_oauth_account(tokens);
        let profile_dir = env.home_dir.join("managed-instance");
        fs::create_dir_all(&profile_dir).expect("create managed instance");
        fs::write(profile_dir.join("auth.json"), "existing-auth").expect("seed existing auth");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(prepare_account_for_injection_from_auth_dir(
                &account.id,
                Some(&profile_dir),
            ))
            .expect("expired id_token must not block runtime projection");

        assert_eq!(prepared.id, account.id);
        let projected =
            fs::read_to_string(profile_dir.join("auth.json")).expect("read projected auth");
        assert!(projected.contains(&account.tokens.access_token));
        let persisted = load_account(&account.id).expect("load account");
        assert!(!persisted.requires_reauth);
    }

    #[test]
    fn valid_access_token_is_not_blocked_by_previous_refresh_token_failure() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-auth-reused-rt-fallback-test");
        let mut tokens = make_codex_tokens(
            "guard-fallback@example.com",
            "acc-guard-fallback",
            "org-guard-fallback",
            "guard-fallback",
            "rt-reused",
        );
        tokens.id_token = make_jwt(serde_json::json!({
            "exp": 1i64,
            "email": "guard-fallback@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-guard-fallback",
                "chatgpt_user_id": "user-guard-fallback",
                "chatgpt_plan_type": "plus",
                "poid": "org-guard-fallback"
            }
        }));
        let mut account = seed_oauth_account(tokens);
        account.requires_reauth = true;
        account.reauth_reason = Some(format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        ));
        save_account(&account).expect("save reused RT account");

        let profile_dir = env.home_dir.join("guarded-instance");
        fs::create_dir_all(&profile_dir).expect("create guarded instance");
        fs::write(profile_dir.join("auth.json"), "existing-auth").expect("seed existing auth");
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let prepared = runtime
            .block_on(prepare_account_for_injection_from_auth_dir(
                &account.id,
                Some(&profile_dir),
            ))
            .expect("valid access_token should remain projectable");
        assert_eq!(prepared.id, account.id);
        assert!(fs::read_to_string(profile_dir.join("auth.json"))
            .expect("read projected auth")
            .contains(&account.tokens.access_token));

        let repeated = runtime
            .block_on(prepare_account_for_injection_from_auth_dir(&account.id, Some(&profile_dir)))
            .expect("repeated projection should preserve access_token-only validation");
        assert_eq!(repeated.id, account.id);
        let persisted = load_account(&account.id).expect("load persisted reused RT account");
        assert!(!persisted.requires_reauth);
        assert_eq!(persisted.reauth_reason, None);
        assert!(persisted.quota_error.is_none());
        assert_eq!(persisted.tokens.id_token, account.tokens.id_token);
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
        assert_eq!(persisted.tokens.refresh_token, account.tokens.refresh_token);
        assert_eq!(persisted.token_generation, account.token_generation);
    }

    #[test]
    fn expired_access_token_projection_is_rejected() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-auth-expired-at-fallback-test");
        let mut tokens = make_codex_tokens(
            "guard-expired@example.com",
            "acc-guard-expired",
            "org-guard-expired",
            "guard-expired",
            "rt-reused",
        );
        tokens.id_token = make_jwt(serde_json::json!({ "exp": 1i64 }));
        tokens.access_token = make_jwt(serde_json::json!({ "exp": 1i64 }));
        let mut account = seed_oauth_account(tokens);
        account.requires_reauth = true;
        account.reauth_reason = Some(format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        ));
        save_account(&account).expect("save expired AT account");

        let profile_dir = env.home_dir.join("guarded-expired-instance");
        fs::create_dir_all(&profile_dir).expect("create guarded expired instance");
        fs::write(profile_dir.join("auth.json"), "existing-auth").expect("seed existing auth");
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(prepare_account_for_injection_from_auth_dir(&account.id, Some(&profile_dir)))
            .expect_err("expired access token must block projection");

        assert!(
            !error.contains("refresh_token_reused"),
            "refresh_token_reused must not be exposed as an account failure: {error}"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("auth.json")).expect("read unchanged auth"),
            "existing-auth"
        );
    }

    #[test]
    fn client_refresh_preparation_allows_valid_access_token_after_previous_rt_failure() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-auth-switch-reused-rt-test");
        let mut tokens = make_codex_tokens(
            "guard-switch@example.com",
            "acc-guard-switch",
            "org-guard-switch",
            "guard-switch",
            "rt-reused",
        );
        tokens.id_token = make_jwt(serde_json::json!({
            "exp": 1i64,
            "email": "guard-switch@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-guard-switch",
                "chatgpt_user_id": "user-guard-switch",
                "chatgpt_plan_type": "plus",
                "poid": "org-guard-switch"
            }
        }));
        let mut account = seed_oauth_account(tokens);
        account.requires_reauth = true;
        account.reauth_reason = Some(format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        ));
        save_account(&account).expect("save reused RT switch account");
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(super::refresh_managed_account_locked(
                &account.id,
                false,
                "switch",
                None,
                true,
                false,
            ))
            .expect("expired id_token alone must not block a valid access_token");
        assert_eq!(prepared.id, account.id);
        let persisted = load_account(&account.id).expect("load switched reused RT account");
        assert!(!persisted.requires_reauth);
        assert_eq!(persisted.reauth_reason, None);
        assert_eq!(persisted.tokens.id_token, account.tokens.id_token);
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
        assert_eq!(persisted.tokens.refresh_token, account.tokens.refresh_token);
        assert_eq!(persisted.token_generation, account.token_generation);
    }

    #[test]
    fn force_refresh_reuses_newer_generation_without_network_refresh() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-force-refresh-generation-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "newer-generation",
            "rt-newer-generation",
        ));
        account.token_generation = 2;
        account.token_updated_at = Some(now_timestamp());
        save_account(&account).expect("save newer generation account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let refreshed = runtime
            .block_on(force_refresh_managed_account_after_observed(
                &account.id,
                1,
                "test observed generation",
            ))
            .expect("newer generation should be reused");

        assert_eq!(refreshed.token_generation, 2);
        assert_eq!(refreshed.tokens.access_token, account.tokens.access_token);
        assert_eq!(
            refreshed.tokens.refresh_token.as_deref(),
            account.tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn missing_refresh_token_reauth_is_cleared_for_access_token_only_accounts() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-only-reauth-clear-test");
        let mut tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "access-only",
            "rt-unused",
        );
        tokens.refresh_token = None;
        let mut account = seed_oauth_account(tokens);
        account.requires_reauth = true;
        account.reauth_reason = Some(
            "Codex 登录授权缺少 refresh_token，无法自动续期；当前 access_token 已不可用。"
                .to_string(),
        );
        save_account(&account).expect("save access-token-only reauth account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(ensure_managed_account_fresh(&account.id))
            .expect("access-token-only account should remain usable");

        assert!(!prepared.requires_reauth);
        assert_eq!(prepared.tokens.refresh_token, None);
        let persisted = load_account(&account.id).expect("persisted account");
        assert!(!persisted.requires_reauth);
        assert_eq!(persisted.reauth_reason, None);
    }

    #[test]
    fn expired_access_token_only_account_requires_reauth_on_prepare() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-only-expired-test");
        let mut tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "access-only-expired",
            "rt-unused",
        );
        tokens.access_token = make_jwt(serde_json::json!({
            "sub": "access-only-expired",
            "exp": 1i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-current",
                "organization_id": "org-current",
            }
        }));
        tokens.refresh_token = None;
        let account = seed_oauth_account(tokens);

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(ensure_managed_account_fresh(&account.id))
            .expect_err("expired access-token-only account should require reauth");

        assert!(error.contains("缺少 refresh_token"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert!(persisted.requires_reauth);
        assert!(persisted
            .reauth_reason
            .as_deref()
            .unwrap_or_default()
            .contains("缺少 refresh_token"));
    }

    #[test]
    fn authority_snapshot_requires_newer_refresh_marker() {
        let mut account = CodexAccount::new(
            "codex_test".to_string(),
            "demo@example.com".to_string(),
            make_codex_tokens(
                "demo@example.com",
                "acc-current",
                "org-current",
                "old",
                "rt-old",
            ),
        );
        account.account_id = Some("acc-current".to_string());
        account.organization_id = Some("org-current".to_string());
        account.token_updated_at = Some(2000);

        let snapshot = LocalCodexOAuthSnapshot {
            tokens: make_codex_tokens(
                "demo@example.com",
                "acc-current",
                "org-current",
                "new",
                "rt-new",
            ),
            email: "demo@example.com".to_string(),
            user_id: Some("user-current".to_string()),
            subscription_active_until: None,
            account_id: Some("acc-current".to_string()),
            organization_id: Some("org-current".to_string()),
            last_refresh_at: Some(1000),
        };
        assert!(!should_accept_authority_snapshot(&account, &snapshot));

        let newer_snapshot = LocalCodexOAuthSnapshot {
            last_refresh_at: Some(3000),
            ..snapshot
        };
        assert!(should_accept_authority_snapshot(&account, &newer_snapshot));
    }

    #[test]
    fn authority_snapshot_with_newer_marker_but_older_access_token_is_rejected() {
        let mut account = CodexAccount::new(
            "codex_test_monotonic".to_string(),
            "demo@example.com".to_string(),
            make_codex_tokens(
                "demo@example.com",
                "acc-current",
                "org-current",
                "current",
                "rt-current",
            ),
        );
        account.account_id = Some("acc-current".to_string());
        account.organization_id = Some("org-current".to_string());
        account.tokens.access_token = make_jwt(serde_json::json!({
            "exp": 20_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        account.token_updated_at = Some(2_000);

        let mut snapshot_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "older",
            "rt-older",
        );
        snapshot_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 10_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let snapshot = LocalCodexOAuthSnapshot {
            tokens: snapshot_tokens,
            email: "demo@example.com".to_string(),
            user_id: Some("user-current".to_string()),
            subscription_active_until: None,
            account_id: Some("acc-current".to_string()),
            organization_id: Some("org-current".to_string()),
            last_refresh_at: Some(3_000),
        };

        assert!(!should_accept_authority_snapshot(&account, &snapshot));
    }

    #[test]
    fn runtime_snapshot_freshness_prefers_latest_official_refresh() {
        let mut older_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "older-runtime",
            "rt-older-runtime",
        );
        older_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 10_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let older = LocalCodexOAuthSnapshot {
            tokens: older_tokens,
            email: "demo@example.com".to_string(),
            user_id: Some("user-current".to_string()),
            subscription_active_until: None,
            account_id: Some("acc-current".to_string()),
            organization_id: Some("org-current".to_string()),
            last_refresh_at: Some(2_000),
        };
        let mut newer_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "newer-runtime",
            "rt-newer-runtime",
        );
        newer_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 20_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let newer = LocalCodexOAuthSnapshot {
            tokens: newer_tokens,
            // 即使运行态文件的 last_refresh 标记较旧，也优先采用有效期更晚的 access_token。
            last_refresh_at: Some(1_000),
            ..older.clone()
        };

        assert!(
            super::local_oauth_snapshot_freshness_key(&newer)
                > super::local_oauth_snapshot_freshness_key(&older)
        );
    }

    #[test]
    fn runtime_authority_sync_writes_only_the_newest_running_profile() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-runtime-authority-selection-test");
        let mut stored_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "stored",
            "rt-stored",
        );
        stored_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 5_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let account = seed_oauth_account(stored_tokens);

        let mut older_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "older-runtime",
            "rt-older-runtime",
        );
        older_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 10_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let older_dir = env.codex_home().join("instance-older");
        write_oauth_auth_file(&older_dir, &older_tokens, "acc-current");

        let mut newer_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "newer-runtime",
            "rt-newer-runtime",
        );
        newer_tokens.access_token = make_jwt(serde_json::json!({
            "exp": 20_000i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        let newer_dir = env.codex_home().join("instance-newer");
        write_oauth_auth_file(&newer_dir, &newer_tokens, "acc-current");

        assert!(super::sync_account_from_runtime_authority_dirs(
            &account.id,
            &[older_dir, newer_dir]
        )
        .expect("sync newest runtime authority"));
        let persisted = load_account(&account.id).expect("load synced account");
        assert_eq!(persisted.tokens.access_token, newer_tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            newer_tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn reauth_consumer_sync_keeps_running_official_profile_unchanged() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-reauth-local-store-only-test");
        let account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "reauth-new",
            "rt-reauth-new",
        ));
        let runtime_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "runtime-current",
            "rt-runtime-current",
        );
        write_oauth_auth_file(&env.codex_home(), &runtime_tokens, "acc-current");

        tokio::runtime::Runtime::new()
            .expect("create runtime")
            .block_on(super::sync_bound_oauth_consumers_after_reauth(&account.id))
            .expect("sync reauthorized local consumers");

        let persisted_runtime =
            super::load_local_oauth_snapshot_from_official_store(&env.codex_home())
                .expect("official runtime snapshot");
        assert_eq!(
            persisted_runtime.tokens.access_token,
            runtime_tokens.access_token
        );
        assert_eq!(
            persisted_runtime.tokens.refresh_token,
            runtime_tokens.refresh_token
        );
        let persisted_account = load_account(&account.id).expect("local Cockpit account");
        assert_eq!(
            persisted_account.tokens.access_token,
            account.tokens.access_token
        );
    }

    #[test]
    fn default_auth_store_prefers_auth_json_over_keychain() {
        let base_dir = make_temp_dir("codex-auth-store-file-priority-test");
        let file_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "file",
            "rt-file",
        );
        let keychain_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "keychain",
            "rt-keychain",
        );
        write_oauth_auth_file(&base_dir, &file_tokens, "acc-current");

        let snapshot = super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(
            &base_dir,
            |_| Ok(Some(build_oauth_auth_file(&keychain_tokens, "acc-current"))),
        )
        .expect("file auth snapshot");

        assert_eq!(snapshot.tokens.access_token, file_tokens.access_token);
        assert_eq!(snapshot.tokens.refresh_token.as_deref(), Some("rt-file"));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn explicit_keyring_auth_store_prefers_keychain() {
        let base_dir = make_temp_dir("codex-auth-store-keyring-priority-test");
        let file_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "file",
            "rt-file",
        );
        let keychain_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "keychain",
            "rt-keychain",
        );
        write_oauth_auth_file(&base_dir, &file_tokens, "acc-current");
        fs::write(
            base_dir.join("config.toml"),
            "cli_auth_credentials_store = \"keyring\"\n",
        )
        .expect("write keyring config");

        let snapshot = super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(
            &base_dir,
            |_| Ok(Some(build_oauth_auth_file(&keychain_tokens, "acc-current"))),
        )
        .expect("keychain auth snapshot");

        assert_eq!(snapshot.tokens.access_token, keychain_tokens.access_token);
        assert_eq!(
            snapshot.tokens.refresh_token.as_deref(),
            Some("rt-keychain")
        );
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn switch_presync_persists_current_rotated_refresh_token_before_overwrite() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-switch-presync-current-auth-test");
        let mut current = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "stored",
            "rt-stored",
        ));
        current.token_updated_at = Some(1);
        save_account(&current).expect("make stored credential older than official refresh");
        let target = upsert_account(make_codex_tokens(
            "target@example.com",
            "acc-target",
            "org-target",
            "target",
            "rt-target",
        ))
        .expect("seed target account");
        assert_ne!(target.id, current.id);
        assert_eq!(
            load_account_index().current_account_id.as_deref(),
            Some(current.id.as_str())
        );

        let rotated_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "rotated",
            "rt-rotated",
        );
        write_oauth_auth_file(&env.codex_home(), &rotated_tokens, "acc-current");

        assert!(
            super::sync_account_from_runtime_authority_dirs(&current.id, &[env.codex_home()])
                .expect("sync active official account")
        );
        let persisted = load_account(&current.id).expect("load current account after presync");
        assert_eq!(persisted.tokens.access_token, rotated_tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            Some("rt-rotated")
        );
    }

    #[test]
    fn detect_auth_file_plan_type_from_filename() {
        let prolite = detect_auth_file_plan_type_from_path(std::path::Path::new(
            "/tmp/codex-demo@example.com-prolite.json",
        ));
        let promax = detect_auth_file_plan_type_from_path(std::path::Path::new(
            "/tmp/codex-demo@example.com-pro-max.json",
        ));
        let team =
            detect_auth_file_plan_type_from_path(std::path::Path::new("/tmp/codex-demo-team.json"));

        assert_eq!(prolite.as_deref(), Some("prolite"));
        assert_eq!(promax.as_deref(), Some("promax"));
        assert_eq!(team, None);
    }
