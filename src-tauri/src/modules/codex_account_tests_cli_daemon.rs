// Exercise real credential writes and failures without starting Codex or using real accounts.
mod codex_cli_daemon_commit_tests {
    use super::*;
    use std::future::Future;
    use std::path::PathBuf;

    fn target_account() -> CodexAccount {
        seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "daemon-switch",
            "rt-daemon-test",
        ))
    }

    async fn switch_with_tracking<F, Fut>(
        account: &CodexAccount,
        reauth: bool,
        before_commit: F,
        committed_home: &mut Option<PathBuf>,
    ) -> Result<CodexAccount, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        if reauth {
            super::super::switch_account_managed_after_reauth_with_before_commit_options(
                &account.id,
                account.token_generation,
                before_commit,
                committed_home,
            )
            .await
        } else {
            super::super::switch_account_managed_with_before_commit_and_revalidation_options(
                &account.id,
                before_commit,
                committed_home,
            )
            .await
        }
    }

    fn block_file_write(path: &Path) {
        if path.is_file() {
            fs::remove_file(path).expect("remove test file before blocking replacement");
        }
        fs::create_dir(path).expect("directory blocks atomic file replacement on every platform");
    }

    fn assert_projected_account(home: &Path, account: &CodexAccount) {
        let auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(
            auth.pointer("/tokens/access_token")
                .and_then(|value| value.as_str()),
            Some(account.tokens.access_token.as_str()),
        );
    }

    #[test]
    fn committed_home_survives_config_projection_index_and_account_write_failures() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for reauth in [false, true] {
            for failure in ["config", "projection", "index", "account"] {
                let env = TestEnvGuard::new("codex-daemon-post-auth-failure");
                let account = target_account();
                let blocked_path = match failure {
                    "config" => env.codex_home().join("config.toml"),
                    "projection" => env
                        .codex_home()
                        .join(super::super::CODEX_AUTH_PROJECTION_FILE_NAME),
                    "index" => get_accounts_storage_path(),
                    "account" => get_accounts_dir().join(format!("{}.json", account.id)),
                    _ => unreachable!(),
                };
                let expected_error_path = blocked_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let mut committed_home = None;
                let result = runtime.block_on(switch_with_tracking(
                    &account,
                    reauth,
                    || async {
                        block_file_write(&blocked_path);
                        Ok(())
                    },
                    &mut committed_home,
                ));
                let error = result.expect_err("post-auth write must fail");
                assert!(
                    error.contains(&expected_error_path),
                    "{failure}, reauth={reauth}: {error}"
                );
                assert_eq!(
                    committed_home,
                    Some(env.codex_home()),
                    "{failure}, reauth={reauth}"
                );
                assert_projected_account(&env.codex_home(), &account);
            }
        }
    }

    #[test]
    fn failed_auth_write_or_cancel_before_commit_does_not_request_restart() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for reauth in [false, true] {
            for cancel in [false, true] {
                let env = TestEnvGuard::new("codex-daemon-before-auth-failure");
                let account = target_account();
                let auth_path = env.codex_home().join("auth.json");
                let old_auth = "{\"sentinel\":\"old-auth\"}";
                fs::write(&auth_path, old_auth).unwrap();
                // Reusing a caller's output must not retain a previous switch's home.
                let mut committed_home = Some(PathBuf::from("previous-profile"));
                let result = runtime.block_on(switch_with_tracking(
                    &account,
                    reauth,
                    || async {
                        if cancel {
                            return Err("CODEX_START_CANCELLED".to_string());
                        }
                        block_file_write(&auth_path);
                        Ok(())
                    },
                    &mut committed_home,
                ));
                let error = result.expect_err("switch must fail before auth is written");
                assert_eq!(committed_home, None);
                if cancel {
                    assert_eq!(error, "CODEX_START_CANCELLED");
                    assert_eq!(fs::read_to_string(auth_path).unwrap(), old_auth);
                } else {
                    assert!(error.contains("auth.json"), "{error}");
                    assert!(auth_path.is_dir());
                }
            }
        }
    }

    #[test]
    fn preparation_failure_clears_committed_home_without_stopping_runtime() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for reauth in [false, true] {
            let env = TestEnvGuard::new("codex-daemon-prepare-failure");
            let mut account = target_account();
            account.id = "missing-account".to_string();
            let mut committed_home = Some(PathBuf::from("previous-profile"));
            let result = runtime.block_on(switch_with_tracking(
                &account,
                reauth,
                || async { panic!("preparation failed before runtime should be stopped") },
                &mut committed_home,
            ));
            assert!(result.is_err());
            assert_eq!(committed_home, None);
            assert!(!env.codex_home().join("auth.json").exists());
        }
    }

    #[test]
    fn successful_switch_records_the_home_for_both_normal_and_reauth_flows() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for reauth in [false, true] {
            let env = TestEnvGuard::new("codex-daemon-switch-success");
            let account = target_account();
            let mut committed_home = None;
            let switched = runtime
                .block_on(switch_with_tracking(
                    &account,
                    reauth,
                    || async { Ok(()) },
                    &mut committed_home,
                ))
                .expect("switch succeeds");
            assert_eq!(switched.id, account.id);
            assert_eq!(committed_home, Some(env.codex_home()));
            assert_projected_account(&env.codex_home(), &account);
            assert_eq!(load_account_index().current_account_id, Some(account.id));
        }
    }

    #[test]
    fn bound_oauth_commit_keeps_home_when_index_write_fails() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-daemon-bound-oauth-failure");
        let oauth = target_account();
        let mut api_key = CodexAccount::new_api_key(
            "api-daemon-test".to_string(),
            "api@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
            Vec::new(),
        );
        api_key.bound_oauth_account_id = Some(oauth.id.clone());
        let id = api_key.id.clone();
        block_file_write(&get_accounts_storage_path());
        let mut committed_home = None;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(super::super::commit_account_switch_locked(
                &id,
                super::super::PreparedCodexAccountSwitch::ApiKeyWithOauth {
                    api_key_account: api_key,
                    oauth_account: oauth.clone(),
                },
                &mut committed_home,
            ))
            .expect_err("index replacement fails after bound OAuth credentials are written");
        assert!(error.contains("codex_accounts.json"), "{error}");
        assert_eq!(committed_home, Some(env.codex_home()));
        assert_projected_account(&env.codex_home(), &oauth);
    }
}
