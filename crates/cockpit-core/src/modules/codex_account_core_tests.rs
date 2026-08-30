// cockpit-core Codex 账号测试：凭据、切号、导入和配额行为。
// 测试内容作为原 tests 模块内部实现被 include。
    use super::{
        build_account_storage_id, build_auth_file_value, decode_jwt_payload_value,
        ensure_managed_account_fresh, extract_codex_import_candidate_from_value,
        extract_codex_tokens_from_value, force_refresh_managed_account, get_accounts_dir,
        get_accounts_storage_path, get_current_account, list_accounts_checked, load_account,
        load_account_index, looks_like_sub2api_export, merge_existing_auth_file_value,
        read_api_provider_from_config_toml, read_quick_config_from_config_toml,
        resolve_api_provider_config, save_account, save_account_index, sync_account_from_auth_dir,
        sync_api_key_account_from_local_state, sync_managed_projection_from_auth_dir,
        upsert_account, upsert_account_for_reauth, upsert_account_from_access_token,
        upsert_account_from_auth_tokens, validate_api_key_credentials, write_account_bundle_to_dir,
        write_api_key_provider_to_config_toml, write_api_provider_to_config_toml,
        write_auth_file_to_dir, write_quick_config_to_config_toml, ApiProviderConfig,
        CodexAccountIndex, CodexAccountSummary, CodexAuthFile, CodexAuthTokens,
        CodexJsonImportCandidate, CODEX_AUTO_COMPACT_DEFAULT_LIMIT, CODEX_CONTEXT_WINDOW_1M_VALUE,
    };
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexTokens};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::fs;
    use std::sync::{LazyLock, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
        previous_home: Option<std::ffi::OsString>,
        previous_codex_home: Option<std::ffi::OsString>,
        previous_data_dir: Option<std::ffi::OsString>,
    }

    impl TestEnvGuard {
        fn new(prefix: &str) -> Self {
            let home_dir = make_temp_dir(prefix);
            let codex_home = home_dir.join(".codex");
            fs::create_dir_all(&codex_home).expect("create codex home");

            let previous_home = std::env::var_os("HOME");
            let previous_codex_home = std::env::var_os("CODEX_HOME");
            let previous_data_dir = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("CODEX_HOME", &codex_home);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &home_dir);

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
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            let _ = fs::remove_dir_all(&self.home_dir);
        }
    }

    #[test]
    fn test_env_guard_isolates_and_restores_cockpit_data_dir() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let previous_data_dir = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
        let isolated_data_dir = {
            let env = TestEnvGuard::new("codex-core-data-dir-guard-test");
            let legacy_data_dir = env.home_dir.join("legacy-codex-data");
            fs::create_dir_all(&legacy_data_dir).expect("create isolated legacy data dir");
            let legacy_index = legacy_data_dir.join("codex_accounts.json");
            fs::write(&legacy_index, "legacy sentinel").expect("write legacy sentinel");

            assert_eq!(
                crate::modules::config::get_data_dir().expect("resolve isolated data dir"),
                env.home_dir
            );
            let accounts_storage_path = get_accounts_storage_path();
            assert_eq!(
                accounts_storage_path,
                env.home_dir.join("codex_accounts.json")
            );
            assert!(legacy_index.exists());
            assert!(!accounts_storage_path.exists());
            env.home_dir.clone()
        };

        assert_eq!(
            std::env::var_os("COCKPIT_TOOLS_DATA_DIR"),
            previous_data_dir
        );
        assert!(!isolated_data_dir.exists());
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
            "https://api.openai.com/auth": {
                "chatgpt_user_id": format!("user-{}", suffix),
                "chatgpt_plan_type": "pro",
                "account_id": account_id,
                "organization_id": organization_id,
            }
        }));
        let access_token = make_jwt(serde_json::json!({
            "sub": format!("access-{}", suffix),
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

    fn seed_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let email = "demo@example.com";
        let account_id = "acc-current";
        let organization_id = "org-current";
        let storage_id = build_account_storage_id(email, Some(account_id), Some(organization_id));

        let mut account = CodexAccount::new(storage_id.clone(), email.to_string(), tokens);
        account.user_id = Some("user-current".to_string());
        account.plan_type = Some("pro".to_string());
        account.account_id = Some(account_id.to_string());
        account.organization_id = Some(organization_id.to_string());
        save_account(&account).expect("save account");

        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: storage_id,
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
        index.current_account_id = Some(account.id.clone());
        save_account_index(&index).expect("save index");

        account
    }

    fn write_oauth_auth_file(base_dir: &std::path::Path, tokens: &CodexTokens, account_id: &str) {
        let auth_file = CodexAuthFile {
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
            last_refresh: Some(serde_json::Value::String(
                "2026-04-13T00:00:00.000000Z".to_string(),
            )),
        };

        fs::create_dir_all(base_dir).expect("create auth dir");
        fs::write(
            base_dir.join("auth.json"),
            serde_json::to_string_pretty(&auth_file).expect("serialize auth file"),
        )
        .expect("write auth file");
    }

    #[test]
    fn build_auth_file_value_keeps_empty_refresh_token_field_for_cpa_accounts() {
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

        assert!(tokens.contains_key("refresh_token"));
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
    fn build_auth_file_value_marks_oauth_as_codex_type_not_api_key() {
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

        let api_key = CodexAccount::new_api_key(
            "codex-api-type".to_string(),
            "api@type.example".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
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
        let base_dir = make_temp_dir("codex-core-auth-merge-write-test");
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
    fn force_refresh_keeps_access_token_only_accounts_usable() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-core-access-token-only-refresh-test");
        let mut tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "access-only",
            "rt-unused",
        );
        tokens.refresh_token = None;
        let account = seed_oauth_account(tokens);

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let refreshed = runtime
            .block_on(force_refresh_managed_account(&account.id, "test"))
            .expect("access-token-only account should skip refresh without failing");

        assert_eq!(refreshed.tokens.refresh_token, None);
    }

    #[test]
    fn stale_missing_refresh_token_reauth_is_cleared() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-core-access-token-only-reauth-clear-test");
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
        account.reauth_reason = Some("Token 已过期且无 refresh_token，请重新登录".to_string());
        save_account(&account).expect("save access-token-only reauth account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(ensure_managed_account_fresh(&account.id))
            .expect("missing-refresh reauth marker should be cleared");

        assert!(!prepared.requires_reauth);
        let persisted = load_account(&account.id).expect("persisted account");
        assert!(!persisted.requires_reauth);
        assert_eq!(persisted.reauth_reason, None);
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
    fn extract_tokens_from_flat_codex_json_falls_back_to_session_token() {
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
        assert_eq!(
            tokens.refresh_token.as_deref(),
            Some("encrypted-session-token")
        );
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
    fn extract_tokens_from_nested_tokens_json_falls_back_to_session_token() {
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
        assert_eq!(
            tokens.refresh_token.as_deref(),
            Some("encrypted-session-token")
        );
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
    }

    #[test]
    fn extract_candidate_from_sub2api_account_credentials() {
        let access_token = make_jwt(serde_json::json!({
            "email": "sub2api@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-sub2api",
                "chatgpt_user_id": "user-sub2api"
            }
        }));
        let value = serde_json::json!({
            "name": "Sub2API account",
            "notes": "imported from sub2api",
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "access_token": access_token
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("Sub2API account should expose access_token");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                account_note,
            } => {
                assert_eq!(account_note.as_deref(), Some("imported from sub2api"));
                assert!(decode_jwt_payload_value(&access_token).is_some());
            }
            _ => panic!("expected accessToken-only candidate"),
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
    fn upsert_access_token_only_account_uses_access_claims() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-import-test");
        let access_token = make_jwt(serde_json::json!({
            "email": "access@example.com",
            "sub": "user-access",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access",
                "chatgpt_user_id": "user-access",
                "chatgpt_plan_type": "team",
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
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
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
    fn upsert_reuses_legacy_email_only_account_when_identity_appears() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-core-legacy-email-only-dedupe-test");
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
    fn reauth_updates_explicit_target_account_even_when_identity_changes() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-core-explicit-reauth-target-test");
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
    fn reauth_removes_generated_duplicate_for_target_identity() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-core-explicit-reauth-dedupe-test");
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
    fn current_account_does_not_sync_tokens_from_official_store() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-current-account-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-old",
        ));
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "latest",
            "rt-latest",
        );
        write_oauth_auth_file(&env.codex_home(), &latest_tokens, "acc-current");

        let current = get_current_account().expect("current account");
        assert_eq!(current.id, stored.id);
        assert_eq!(current.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            current.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );

        let persisted = load_account(&stored.id).expect("persisted account");
        assert_eq!(persisted.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn sync_account_from_auth_dir_updates_store_for_managed_home() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-auth-dir-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_account_from_auth_dir(&stored.id, &managed_home).expect("sync account");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );

        let persisted = load_account(&stored.id).expect("persisted account");
        assert_eq!(persisted.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn managed_projection_sync_requires_projection_marker() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-managed-projection-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        write_account_bundle_to_dir(&managed_home, &stored).expect("write managed projection");

        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_managed_projection_from_auth_dir(&stored.id, &managed_home)
            .expect("sync managed projection");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
        assert!(synced.token_generation > stored.token_generation);
    }

    #[test]
    fn config_toml_uses_openai_base_url_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-openai-base-url-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.example.com/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("openai_base_url = \"https://api.example.com\""));
        assert!(!content.contains("model_provider ="));
        assert!(!content
            .lines()
            .any(|line| line.trim_start().starts_with("base_url =")));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir)
                .base_url
                .as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: Some("https://api.example.com".to_string()),
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_skips_openai_base_url_for_default_official_endpoint() {
        let base_dir = make_temp_dir("codex-config-openai-default-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        if config_path.exists() {
            let content = fs::read_to_string(&config_path).expect("read config");
            assert!(!content.contains("openai_base_url"));
        }
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_cleans_runtime_provider_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-clean-runtime-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "codex_local_access"
openai_base_url = "https://legacy.example.com/v1"

[model_providers.codex_local_access]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-test"

[model_providers.cockpit_api]
name = "Cockpit Api"
base_url = "https://chongcodex.cn/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.openai_api_key]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.user_manual_provider_not_managed]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false
"#,
        )
        .expect("write runtime config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("[model_providers.codex_local_access]"));
        assert!(!content.contains("[model_providers.cockpit_api]"));
        assert!(!content.contains("[model_providers.openai_api_key]"));
        assert!(!content.contains("experimental_bearer_token"));
        assert!(content.contains("[model_providers.user_manual_provider_not_managed]"));
        assert!(!content.contains("openai_base_url"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_uses_model_provider_section_for_custom_provider() {
        let base_dir = make_temp_dir("codex-config-custom-provider-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"relay\""));
        assert!(content.contains("[model_providers.relay]"));
        assert!(content.contains("name = \"Relay\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = false"));
        assert!(content.contains("supports_websockets = false"));
        assert!(!content.contains("openai_base_url"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: Some("relay".to_string()),
                provider_name: Some("Relay".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_uses_builtin_openai_for_responses_relay() {
        let base_dir = make_temp_dir("codex-config-api-key-provider-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("[model_providers."));
        assert!(!content.contains("experimental_bearer_token"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_account_switch_updates_relay_key_and_base_url_together() {
        let base_dir = make_temp_dir("codex-api-key-relay-switch-test");
        let mut first = CodexAccount::new_api_key(
            "relay-a".to_string(),
            "relay-a@example.com".to_string(),
            "sk-relay-a".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay-a.example.com/v1".to_string()),
            Some("relay_a".to_string()),
            Some("Relay A".to_string()),
        );

        write_auth_file_to_dir(&base_dir, &first).expect("write first relay account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read first auth"),
        )
        .expect("parse first auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-relay-a");
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read first config");
        assert!(config.contains("openai_base_url = \"https://relay-a.example.com/v1\""));
        assert!(!config.contains("model_provider = "));

        sync_api_key_account_from_local_state(&mut first, &base_dir);
        assert_eq!(first.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(first.api_provider_id.as_deref(), Some("relay_a"));
        assert_eq!(first.api_provider_name.as_deref(), Some("Relay A"));

        let second = CodexAccount::new_api_key(
            "relay-b".to_string(),
            "relay-b@example.com".to_string(),
            "sk-relay-b".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay-b.example.com/v1".to_string()),
            Some("relay_b".to_string()),
            Some("Relay B".to_string()),
        );
        write_auth_file_to_dir(&base_dir, &second).expect("write second relay account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read second auth"),
        )
        .expect("parse second auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-relay-b");
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read second config");
        assert!(config.contains("openai_base_url = \"https://relay-b.example.com/v1\""));
        assert!(!config.contains("relay-a.example.com"));
        assert!(!config.contains("model_provider = "));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_import_preserves_relay_pair_and_provider_identity() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-core-api-key-import-projection-test");
        let account = CodexAccount::new_api_key(
            "portable-relay".to_string(),
            "portable-relay@example.com".to_string(),
            "sk-core-imported-relay".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://core-imported-relay.example.com/v1".to_string()),
            Some("core_imported_relay".to_string()),
            Some("Core Imported Relay".to_string()),
        );

        let mut imported = super::import_account_struct(account).expect("import API key account");
        assert_eq!(imported.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(
            imported.api_provider_id.as_deref(),
            Some("core_imported_relay")
        );
        assert_eq!(
            imported.api_provider_name.as_deref(),
            Some("Core Imported Relay")
        );

        let profile_dir = env.home_dir.join("imported-relay-profile");
        write_account_bundle_to_dir(&profile_dir, &imported)
            .expect("project imported API key account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read imported auth"),
        )
        .expect("parse imported auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-core-imported-relay");
        let config =
            fs::read_to_string(profile_dir.join("config.toml")).expect("read imported config");
        assert!(config.contains("openai_base_url = \"https://core-imported-relay.example.com/v1\""));
        assert!(!config.contains("model_provider = "));
        assert!(!config.contains("[model_providers.core_imported_relay]"));

        sync_api_key_account_from_local_state(&mut imported, &profile_dir);
        assert_eq!(imported.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(
            imported.api_provider_id.as_deref(),
            Some("core_imported_relay")
        );
        assert_eq!(
            imported.api_provider_name.as_deref(),
            Some("Core Imported Relay")
        );
    }

    #[test]
    fn quick_config_reads_custom_context_window_without_hiding_it() {
        let base_dir = make_temp_dir("codex-quick-config-custom-window-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 200000\nmodel_auto_compact_token_limit = 180000\n",
        )
        .expect("write config");

        let quick_config =
            read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(!quick_config.context_window_1m);
        assert_eq!(quick_config.auto_compact_token_limit, 180000);
        assert_eq!(quick_config.detected_model_context_window, Some(200000));
        assert_eq!(quick_config.detected_auto_compact_token_limit, Some(180000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_enable_1m_context_window() {
        let base_dir = make_temp_dir("codex-quick-config-enable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result = write_quick_config_to_config_toml(&base_dir, true, Some(880000))
            .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("model_auto_compact_token_limit = 880000"));
        assert_eq!(result.context_window_1m, true);
        assert_eq!(result.auto_compact_token_limit, 880000);
        assert_eq!(
            result.detected_model_context_window,
            Some(CODEX_CONTEXT_WINDOW_1M_VALUE)
        );
        assert_eq!(result.detected_auto_compact_token_limit, Some(880000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_remove_managed_fields() {
        let base_dir = make_temp_dir("codex-quick-config-disable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\nmodel = \"gpt-5\"\n",
        )
        .expect("write config");

        let result =
            write_quick_config_to_config_toml(&base_dir, false, None).expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_context_window"));
        assert!(!content.contains("model_auto_compact_token_limit"));
        assert!(content.contains("model = \"gpt-5\""));
        assert!(!result.context_window_1m);
        assert_eq!(
            result.auto_compact_token_limit,
            CODEX_AUTO_COMPACT_DEFAULT_LIMIT
        );
        assert_eq!(result.detected_model_context_window, None);
        assert_eq!(result.detected_auto_compact_token_limit, None);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn validate_api_key_credentials_rejects_url_api_key() {
        let err = validate_api_key_credentials("http://127.0.0.1:3000/v1", None)
            .expect_err("url should be rejected as api key");
        assert!(err.contains("API Key 不能是 URL"));
    }

    #[test]
    fn validate_api_key_credentials_rejects_invalid_base_url() {
        let err = validate_api_key_credentials("sk-test-key", Some("not-a-url"))
            .expect_err("invalid base url should be rejected");
        assert!(err.contains("Base URL 格式无效"));
    }

    #[test]
    fn validate_api_key_credentials_accepts_valid_values() {
        let (api_key, api_base_url) =
            validate_api_key_credentials("  sk-test-key  ", Some("https://relay.local/v1/"))
                .expect("valid api key + base url should pass");
        assert_eq!(api_key, "sk-test-key");
        assert_eq!(api_base_url.as_deref(), Some("https://relay.local/v1"));
    }

    #[test]
    #[ignore = "manual local Codex repair smoke test"]
    fn local_codex_index_repair_smoke() {
        crate::modules::logger::init_logger();

        let index_path = get_accounts_storage_path();
        let accounts_dir = get_accounts_dir();
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 索引路径: {}",
            index_path.display()
        );
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 详情目录: {}",
            accounts_dir.display()
        );

        let accounts = list_accounts_checked().expect("local Codex repair should succeed");
        let index = load_account_index();
        eprintln!(
            "[LocalCodexRepairTest] 修复/读取完成: accounts={}, current_account_id={}",
            accounts.len(),
            index.current_account_id.as_deref().unwrap_or("-")
        );

        if let Ok(log_file) = crate::modules::logger::get_latest_app_log_file() {
            eprintln!(
                "[LocalCodexRepairTest] 应用日志文件: {}",
                log_file.display()
            );
        }
    }
