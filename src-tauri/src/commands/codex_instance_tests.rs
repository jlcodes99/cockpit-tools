mod tests {
    use super::*;
    include!("codex_instance_test_env_ordering.rs");

    struct TestDataDirGuard {
        root: PathBuf,
        previous_test_data_dir: Option<std::ffi::OsString>,
        previous_data_dir: Option<std::ffi::OsString>,
    }

    impl TestDataDirGuard {
        fn new(prefix: &str) -> Self {
            let root = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            let data_dir = root.join("data");
            std::fs::create_dir_all(&data_dir).expect("create test data dir");
            let previous_test_data_dir = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
            let previous_data_dir = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &data_dir);
            Self {
                root,
                previous_test_data_dir,
                previous_data_dir,
            }
        }
    }

    impl Drop for TestDataDirGuard {
        fn drop(&mut self) {
            match self.previous_test_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            match self.previous_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepared_cli_switch_resumes_once_with_the_committed_profile() {
        use modules::codex_session_resume::{self, ResumeSettings, ResumeTarget};
        use std::os::unix::fs::PermissionsExt;

        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("codex-cli-resume-after-switch");
        let data = env.root.join("data");
        let profile = env.root.join("profile");
        let cli = env.root.join("fixture-cli");
        let instance = "fixture-cli-instance";
        let thread = uuid::Uuid::from_u128(1);
        std::fs::create_dir(&profile).unwrap();
        let mut store = modules::codex_instance::load_instance_store().unwrap();
        store.instances.push(InstanceProfile {
            id: instance.into(),
            name: "CLI fixture".into(),
            user_data_dir: profile.to_string_lossy().into_owned(),
            working_dir: Some(env.root.to_string_lossy().into_owned()),
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: InstanceLaunchMode::Cli,
            app_speed: Default::default(),
            created_at: 1,
            last_launched_at: None,
            last_pid: None,
        });
        modules::codex_instance::save_instance_store(&store).unwrap();
        std::fs::write(
            data.join("codex_wakeup_runtime_config.json"),
            serde_json::json!({ "codexCliPath": cli }).to_string(),
        ).unwrap();
        std::fs::write(&cli, format!(r#"#!/bin/sh
case "$1" in
  --version) echo 'codex-cli 1.0.0';;
  app-server)
    read -r initialize
    echo '{{"id":1,"result":{{}}}}'
    read -r initialized
    read -r thread
    echo '{{"id":3,"result":{{"thread":{{"id":"{thread}"}}}}}}'
    read -r close
    ;;
  exec)
    [ "$2" = resume ] && [ "$4" = '{thread}' ] && [ "$5" = - ] || exit 7
    cat "$CODEX_HOME/auth.json" > identity
    cat >> prompt
    ;;
  *) exit 8;;
esac
"#)).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let settings = ResumeSettings {
            enabled: true, target: ResumeTarget::Specific, thread_id: Some(thread),
            prompt: "继续处理\nfixture".into(),
        };
        let profile_lease = modules::codex_account::try_acquire_profile_mutation_lease(
            &profile, "test-prepared-cli-switch",
        ).unwrap();
        let prepared = should_resume_cli_after_start(StartRuntimeState::Stopped, None, None);
        let explicit = should_resume_cli_after_start(
            StartRuntimeState::NeedsStop, Some("fixture-binding"), None,
        );
        assert!(!resume_cli_after_start(prepared, instance, &profile).await.unwrap());
        codex_session_resume::save(instance, &settings).unwrap();
        for skip in [None, Some("prepareCredentials"), Some("writeProfile")] {
            let expected_binding = skip.map(|_| "fixture-binding");
            let preview_only = should_resume_cli_after_start(
                StartRuntimeState::NeedsStop, expected_binding, skip,
            );
            assert!(!resume_cli_after_start(preview_only, instance, &profile).await.unwrap());
        }
        assert!(resume_cli_after_start(explicit, instance, &env.root).await.is_err());
        request_codex_instance_start_cancel(instance);
        let cancelled = resume_cli_after_start(explicit, instance, &profile).await;
        clear_codex_instance_start_cancel(instance);
        assert_eq!(cancelled.unwrap_err(), "CODEX_START_CANCELLED");
        assert!(!env.root.join("prompt").exists());

        let identity = r#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-new"}"#;
        std::fs::write(profile.join("auth.json"), identity).unwrap();
        assert!(resume_cli_after_start(explicit, instance, &profile).await.unwrap());
        drop(profile_lease);
        let database = rusqlite::Connection::open(data.join("codex_session_resume.sqlite")).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let outcome: String = database.query_row(
                    "SELECT outcome FROM resume_attempts WHERE instance_id = ?1",
                    [instance],
                    |row| row.get(0),
                ).unwrap();
                if outcome == "submitted" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();

        let _profile_lease = modules::codex_account::try_acquire_profile_mutation_lease(
            &profile, "test-repeated-cli-switch",
        ).unwrap();
        assert!(!resume_cli_after_start(prepared, instance, &profile).await.unwrap());
        assert_eq!(std::fs::read_to_string(env.root.join("identity")).unwrap(), identity);
        assert_eq!(std::fs::read_to_string(env.root.join("prompt")).unwrap(), settings.prompt);
    }

    #[tokio::test]
    async fn old_automatic_responses_gateway_binding_projects_the_upstream() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("codex-automatic-responses-rollback");
        let profile = env.root.join("profile");
        let upstream = "https://relay.example.invalid/v1";
        let mut account = CodexAccount::new_api_key(
            "relay".into(),
            "relay@example.invalid".into(),
            "fixture-key".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some(upstream.into()),
            Some("relay".into()),
            None,
            Vec::new(),
        );
        account.api_wire_api = Some("responses".into());
        modules::codex_account::save_account(&account).unwrap();
        let old_binding = modules::codex_instance::provider_gateway_bind_account_id(&account.id).unwrap();
        inject_preflighted_bound_account_to_profile(&profile, &old_binding).await.unwrap();
        assert_eq!(modules::codex_account::configured_provider_base_url(&profile).as_deref(), Some(upstream));

        account.api_instance_access_mode = Some("gateway".into());
        modules::codex_account::save_account(&account).unwrap();
        assert_eq!(effective_provider_bind_account_id(&old_binding), old_binding.as_str());
    }

    #[tokio::test]
    async fn fast_switch_target_speed_persists_and_survives_auth_config_restore() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("codex-fast-switch-target");
        let mut store = crate::models::InstanceStore::new();
        for id in ["target", "other"] {
            let dir = env.root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.toml"), "model = 'fixture-model'\n").unwrap();
            store.instances.push(InstanceProfile {
                id: id.to_string(), name: id.to_string(),
                user_data_dir: dir.to_string_lossy().to_string(),
                working_dir: None, extra_args: String::new(), bind_account_id: None,
                model_routing: None, launch_mode: InstanceLaunchMode::App,
                app_speed: CodexAppSpeed::Standard, created_at: 1,
                last_launched_at: None, last_pid: None,
            });
        }
        modules::codex_instance::save_instance_store(&store).unwrap();
        for speed in [CodexAppSpeed::Ultrafast, CodexAppSpeed::Fast, CodexAppSpeed::Standard] {
            codex_update_instance(
                "target".to_string(), None, None, None,
                Some(Some("fixture-account".to_string())), None, None, None,
                Some(speed.clone()), None, Some(true),
            ).await.unwrap();
            let reloaded = modules::codex_instance::load_instance_store().unwrap();
            let target = reloaded.instances.iter().find(|item| item.id == "target").unwrap();
            assert_eq!(target.bind_account_id.as_deref(), Some("fixture-account"));
            assert_eq!(target.app_speed, speed);
            assert_eq!(reloaded.instances.iter().find(|item| item.id == "other").unwrap().app_speed, CodexAppSpeed::Standard);
            let dir = env.root.join("target");
            // Simulate a credential/provider projection restoring the old config.
            // No auth files, real accounts, client processes or model calls are used.
            std::fs::write(dir.join("config.toml"), "model = 'fixture-model'\nservice_tier = 'default'\n").unwrap();
            apply_launch_speed_after_auth(&dir, None, target.app_speed.clone()).await.unwrap();
            assert_eq!(modules::codex_speed::get_app_speed_config_for_dir(&dir).unwrap().speed, speed);
            assert!(std::fs::read_to_string(dir.join("config.toml")).unwrap().contains("fixture-model"));
        }
    }

    #[test]
    fn instance_start_guard_rejects_only_duplicate_instance_starts() {
        let first = CodexInstanceStartGuard::acquire("guard-test-a")
            .expect("first start should acquire the instance guard");
        let duplicate = CodexInstanceStartGuard::acquire("guard-test-a");
        let other = CodexInstanceStartGuard::acquire("guard-test-b")
            .expect("different instances should start independently");

        assert_eq!(
            duplicate.expect_err("duplicate start should be rejected"),
            "该 Codex 实例正在启动，请稍候"
        );
        drop(first);
        CodexInstanceStartGuard::acquire("guard-test-a")
            .expect("the guard should be released when the start finishes");
        drop(other);
    }

    #[test]
    fn instance_start_cancel_request_is_scoped_and_clearable() {
        let instance_id = "cancel-test-instance";
        clear_codex_instance_start_cancel(instance_id);
        assert!(!codex_instance_start_cancelled(instance_id));
        request_codex_instance_start_cancel(instance_id);
        assert!(codex_instance_start_cancelled(instance_id));
        assert!(ensure_codex_instance_start_not_cancelled(instance_id).is_err());
        clear_codex_instance_start_cancel(instance_id);
        assert!(ensure_codex_instance_start_not_cancelled(instance_id).is_ok());
    }

    #[test]
    fn deferred_instance_binding_skips_runtime_credential_application() {
        assert!(!should_apply_instance_binding_immediately(true, Some(true)));
        assert!(!should_apply_instance_binding_immediately(
            false,
            Some(false)
        ));
    }

    #[test]
    fn regular_instance_update_keeps_immediate_binding_compatibility() {
        assert!(should_apply_instance_binding_immediately(true, None));
        assert!(should_apply_instance_binding_immediately(true, Some(false)));
    }

    #[test]
    fn build_launch_credential_change_detects_account_to_api_provider_change() {
        let change =
            build_launch_credential_change(Some("account".to_string()), Some("api".to_string()))
                .expect("provider change should trigger session repair");

        assert_eq!(change.from, "account");
        assert_eq!(change.to, "api");
    }

    #[test]
    fn build_launch_credential_change_detects_api_to_account_change() {
        let change =
            build_launch_credential_change(Some("api".to_string()), Some("account".to_string()))
                .expect("credential type change should trigger session repair");

        assert_eq!(change.from, "api");
        assert_eq!(change.to, "account");
    }

    #[test]
    fn build_launch_credential_change_ignores_same_credential_type() {
        let change =
            build_launch_credential_change(Some("api".to_string()), Some("api".to_string()));

        assert!(change.is_none());
    }

    #[test]
    fn launch_credential_kind_treats_local_access_runtime_as_api() {
        assert_eq!(
            launch_credential_kind_for_bind_account_id("codex_local_access_runtime").as_deref(),
            Some("api")
        );
    }

    #[test]
    fn cli_launch_mode_does_not_manage_a_desktop_runtime() {
        assert!(launch_mode_uses_desktop_runtime(&InstanceLaunchMode::App));
        assert!(!launch_mode_uses_desktop_runtime(&InstanceLaunchMode::Cli));
    }

    #[test]
    fn launch_command_preview_does_not_require_an_initialized_profile() {
        let context = CodexLaunchContext {
            user_data_dir: "/path/that/does/not/exist/codex-home".to_string(),
            working_dir: Some("/path/that/does/not/exist/workspace".to_string()),
            extra_args: "--model gpt-test".to_string(),
        };

        let command = build_launch_command_preview(&context)
            .expect("preview should only format the launch command");

        assert!(command.contains("codex-home"));
        assert!(command.contains("workspace"));
        assert!(command.contains("codex"));
        assert!(command.contains("--model"));
        assert!(command.contains("gpt-test"));
    }

    #[tokio::test]
    async fn created_instance_binding_replaces_copied_source_credentials_before_return() {
        let test_root = std::env::temp_dir().join(format!(
            "cockpit-codex-create-bind-test-{}",
            uuid::Uuid::new_v4()
        ));
        let source_dir = test_root.join("source");
        let target_dir = test_root.join("target");
        std::fs::create_dir_all(&source_dir).expect("create source dir");
        std::fs::write(source_dir.join("auth.json"), "source-account")
            .expect("write source credentials");
        modules::instance_store::copy_dir_recursive(&source_dir, &target_dir)
            .expect("copy source profile");

        let instance = InstanceProfile {
            id: "created-instance".to_string(),
            name: "Created instance".to_string(),
            user_data_dir: target_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: Some("target-account".to_string()),
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
        };

        let view = created_instance_view_after_binding(
            instance,
            |profile_dir, bind_account_id, _model_routing| async move {
                let copied = std::fs::read_to_string(profile_dir.join("auth.json"))
                    .map_err(|error| error.to_string())?;
                assert_eq!(copied, "source-account");
                std::fs::write(profile_dir.join("auth.json"), bind_account_id)
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        )
        .await
        .expect("apply created instance binding");

        assert_eq!(view.bind_account_id.as_deref(), Some("target-account"));
        assert_eq!(
            std::fs::read_to_string(target_dir.join("auth.json")).expect("read target credentials"),
            "target-account"
        );

        let _ = std::fs::remove_dir_all(test_root);
    }

    #[tokio::test]
    async fn mixed_routing_update_restores_instance_store_when_speed_write_fails() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-routing-update-rollback");

        let profile_path = env.root.join("profile");
        std::fs::create_dir_all(profile_path.join("config.toml"))
            .expect("block config.toml with a directory");
        let mut store = crate::models::InstanceStore::new();
        store.instances.push(InstanceProfile {
            id: "routing-rollback-instance".to_string(),
            name: "Original name".to_string(),
            user_data_dir: profile_path.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: "--original".to_string(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 1,
            last_launched_at: None,
            last_pid: None,
        });
        modules::codex_instance::save_instance_store(&store).expect("save instance store");

        let result = codex_update_instance(
            "routing-rollback-instance".to_string(),
            Some("Changed name".to_string()),
            None,
            Some("--changed".to_string()),
            None,
            Some(Some(CodexInstanceModelRouting::default())),
            None,
            None,
            Some(CodexAppSpeed::Fast),
            None,
            None,
        )
        .await;

        let error = result.expect_err("speed write should fail for a directory at config.toml");
        assert!(error.contains("Codex config.toml"), "{error}");
        let restored = modules::codex_instance::load_instance_store()
            .expect("load restored store")
            .instances
            .into_iter()
            .find(|instance| instance.id == "routing-rollback-instance")
            .expect("restored instance");
        assert_eq!(restored.name, "Original name");
        assert_eq!(restored.extra_args, "--original");
        assert_eq!(restored.model_routing, None);
        assert_eq!(restored.app_speed, CodexAppSpeed::Standard);
    }

    fn save_instance_with_quick_config(
        env: &TestDataDirGuard,
        instance_id: &str,
        context_window: Option<i64>,
        auto_compact_token_limit: Option<i64>,
    ) -> PathBuf {
        let profile_dir = env.root.join(instance_id);
        std::fs::create_dir_all(&profile_dir).expect("create profile dir");
        std::fs::write(profile_dir.join("config.toml"), "model = \"gpt-5\"\n")
            .expect("write base config");
        modules::codex_account::save_quick_config_for_base_dir_with_default(
            &profile_dir,
            context_window,
            auto_compact_token_limit,
            Some(false),
            Some(test_experimental_models()),
            None,
        )
        .expect("write context config");

        let mut store = crate::models::InstanceStore::new();
        store.instances.push(InstanceProfile {
            id: instance_id.to_string(),
            name: "Context preservation test".to_string(),
            user_data_dir: profile_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 1,
            last_launched_at: None,
            last_pid: None,
        });
        modules::codex_instance::save_instance_store(&store).expect("save instance store");
        profile_dir
    }

    fn test_experimental_models() -> Vec<CodexExperimentalModelDefinition> {
        vec![CodexExperimentalModelDefinition {
            model_id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            reasoning_efforts: None,
            default_reasoning_effort: None,
            ultra_reasoning_effort: None,
            context_window: None,
            auto_compact_token_limit: None,
        }]
    }

    #[test]
    fn app_exit_preserves_running_profiles_even_when_routing_was_disabled_for_later() {
        let profile = |id: &str, enabled: bool, pid: u32| InstanceProfile {
            id: id.to_string(),
            name: id.to_string(),
            user_data_dir: format!("/test/{id}"),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: Some(CodexInstanceModelRouting {
                enabled,
                ..Default::default()
            }),
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 1,
            last_launched_at: None,
            last_pid: Some(pid),
        };
        let profiles = vec![
            profile("active", true, 11),
            profile("disabled-for-later", false, 12),
            profile("stopped", true, 13),
        ];
        let idle = idle_codex_profile_dirs_for_app_exit(
            Some(PathBuf::from("/test/default")),
            Some(10),
            profiles.clone(),
            |pid, _| matches!(pid, Some(10 | 11 | 12)),
        );
        assert_eq!(idle, vec![PathBuf::from("/test/stopped")]);
        let all_idle = idle_codex_profile_dirs_for_app_exit(
            Some(PathBuf::from("/test/default")),
            Some(10),
            profiles,
            |_, _| false,
        );
        assert_eq!(all_idle.len(), 4);
        assert_eq!(all_idle[0], PathBuf::from("/test/default"));
    }

    #[tokio::test]
    async fn instance_configuration_disable_routing_survives_reload_and_binding_update() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-disable-routing");
        let instance_id = "disable-routing-instance";
        let profile_dir =
            save_instance_with_quick_config(&env, instance_id, Some(1_000_000), Some(900_000));
        let before_config = std::fs::read(profile_dir.join("config.toml")).unwrap();
        let mut store = modules::codex_instance::load_instance_store().expect("load store");
        store.instances[0].model_routing = Some(CodexInstanceModelRouting {
            enabled: true,
            ..Default::default()
        });
        modules::codex_instance::save_instance_store(&store).expect("seed enabled routing");

        // Match command argument decoding: null is not an explicit disable.
        let omitted: Option<Option<CodexInstanceModelRouting>> =
            serde_json::from_value(serde_json::Value::Null).expect("decode null");
        assert!(omitted.is_none());
        let disabled: Option<Option<CodexInstanceModelRouting>> =
            serde_json::from_value(serde_json::json!({
                "enabled": false, "version": 1, "routes": []
            }))
            .expect("decode explicit disable");
        let saved = codex_save_instance_configuration(
            instance_id.to_string(),
            None, None, None, None,
            disabled,
            None, None, None, None,
            Some(true),
            None, None, None,
            false,
            test_experimental_models(),
            None,
        )
        .await
        .expect("disable routing");
        assert!(!saved.instance.model_routing.expect("saved routing").enabled);
        assert_eq!(std::fs::read(profile_dir.join("config.toml")).unwrap(), before_config);
        assert!(profile_dir.join(PENDING_MODEL_CATALOG_FILE).exists());

        // A later account selection must preserve the disabled state.
        codex_update_instance(
            instance_id.to_string(),
            None, None, None,
            Some(Some("another-oauth-account".to_string())),
            None, None, None, None, None,
            Some(true),
        )
        .await
        .expect("save new binding without launching Codex");
        let reloaded = modules::codex_instance::load_instance_store().expect("reload store");
        let instance = &reloaded.instances[0];
        assert!(!instance.model_routing.as_ref().expect("persisted routing").enabled);
        assert_eq!(instance.bind_account_id.as_deref(), Some("another-oauth-account"));
        let config = std::fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_context_window = 1000000"));
        assert!(config.contains("model_auto_compact_token_limit = 900000"));
        apply_pending_model_catalog(&profile_dir).expect("apply at next startup");
        assert!(!profile_dir.join(PENDING_MODEL_CATALOG_FILE).exists());
    }

    #[test]
    fn pending_catalog_preserves_active_files_and_handles_removed_default() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let env = TestDataDirGuard::new("pending-catalog");
        let profile = save_instance_with_quick_config(&env, "pending", Some(1_000_000), Some(900_000));
        let before = std::fs::read(profile.join("config.toml")).unwrap();
        let view = save_pending_model_catalog(&profile, true, test_experimental_models(), Some("cpa/gpt-6-astra".into())).unwrap();
        assert!(view.experimental_model_catalog_default_model_id.is_none());
        assert_eq!(std::fs::read(profile.join("config.toml")).unwrap(), before);
        assert!(read_pending_model_catalog(&profile).unwrap().unwrap().enabled);
        apply_pending_model_catalog(&profile).unwrap();
        assert!(read_pending_model_catalog(&profile).unwrap().is_none());
        let config = std::fs::read_to_string(profile.join("config.toml")).unwrap();
        assert!(config.contains("model_context_window = 1000000"));
        assert!(!config.contains("cpa/gpt-6-astra"));
        save_pending_model_catalog(&profile, false, Vec::new(), None).unwrap();
        apply_pending_model_catalog(&profile).unwrap();
        let config = std::fs::read_to_string(profile.join("config.toml")).unwrap();
        assert!(!config.contains("model_catalog_json"));
    }

    #[test]
    fn invalid_pending_catalog_does_not_replace_saved_draft() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let env = TestDataDirGuard::new("pending-invalid");
        let profile = save_instance_with_quick_config(&env, "pending", None, None);
        save_pending_model_catalog(&profile, true, test_experimental_models(), None).unwrap();
        let path = profile.join(PENDING_MODEL_CATALOG_FILE);
        let before = std::fs::read(&path).unwrap();
        assert!(save_pending_model_catalog(&profile, true, Vec::new(), None).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[tokio::test]
    async fn routing_without_oauth_binding_is_auto_disabled_on_save() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let env = TestDataDirGuard::new("pending-rollback");
        let profile = save_instance_with_quick_config(&env, "pending", None, None);
        save_pending_model_catalog(&profile, false, Vec::new(), None).unwrap();
        let result = codex_save_instance_configuration(
            "pending".into(), None, None, None, None,
            Some(Some(CodexInstanceModelRouting { enabled: true, ..Default::default() })),
            None, None, None, None, Some(true), None, None, None,
            true, test_experimental_models(), None,
        ).await;
        let saved = result
            .expect("绑定账号不是 OAuth 订阅账号时，混合模型路由必须自动关闭而不是拦住保存");
        let routing = saved.instance.model_routing.expect("路由配置需要保留");
        assert!(
            !routing.enabled,
            "绑定账号不是可直接登录的 OAuth 订阅账号时必须自动关闭路由"
        );
        assert!(
            read_pending_model_catalog(&profile).unwrap().is_some(),
            "待生效的模型配置不受影响"
        );
    }

    #[tokio::test]
    async fn deleted_account_disables_model_routing_referencing_it() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-routing-deleted-account");
        let base_instance_id = "routing-base-account-instance";
        let route_instance_id = "routing-route-account-instance";

        let route = crate::models::CodexInstanceApiRoute {
            id: "route-1".to_string(),
            namespace: "cpa".to_string(),
            provider_account_id: "deleted-route-account".to_string(),
            enabled: true,
            selected_models: None,
            extra_models: None,
        };
        let mut store = crate::models::InstanceStore::new();
        for (instance_id, bind_account_id) in [
            (base_instance_id, "deleted-oauth-account"),
            (route_instance_id, "kept-oauth-account"),
        ] {
            let profile_dir = env.root.join(instance_id);
            std::fs::create_dir_all(&profile_dir).expect("create profile dir");
            std::fs::write(profile_dir.join("config.toml"), "model = \"gpt-5\"\n")
                .expect("write base config");
            store.instances.push(InstanceProfile {
                id: instance_id.to_string(),
                name: instance_id.to_string(),
                user_data_dir: profile_dir.to_string_lossy().to_string(),
                working_dir: None,
                extra_args: String::new(),
                bind_account_id: Some(bind_account_id.to_string()),
                model_routing: Some(CodexInstanceModelRouting {
                    enabled: true,
                    version: 1,
                    routes: vec![route.clone()],
                }),
                launch_mode: InstanceLaunchMode::App,
                app_speed: CodexAppSpeed::Standard,
                created_at: 1,
                last_launched_at: None,
                last_pid: None,
            });
        }
        modules::codex_instance::save_instance_store(&store).expect("save instance store");

        let affected = modules::codex_instance::disable_model_routing_for_deleted_accounts(&[
            "deleted-oauth-account".to_string(),
            "deleted-route-account".to_string(),
        ])
        .expect("disable routing for deleted accounts");
        assert_eq!(
            affected.len(),
            2,
            "底座账号与路由账号被删除的实例都要自动关闭路由"
        );

        let store = modules::codex_instance::load_instance_store().expect("load instance store");
        for instance_id in [base_instance_id, route_instance_id] {
            let routing = store
                .instances
                .iter()
                .find(|item| item.id == instance_id)
                .and_then(|item| item.model_routing.clone())
                .expect("routing must stay persisted");
            assert!(!routing.enabled, "路由必须自动关闭");
            assert_eq!(routing.routes.len(), 1, "渠道配置必须保留");
        }
    }

    #[tokio::test]
    async fn instance_configuration_save_updates_context_override_with_routing() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-configuration-context-preservation");
        let instance_id = "context-preservation-instance";
        let profile_dir =
            save_instance_with_quick_config(&env, instance_id, Some(1_000_000), Some(900_000));

        let saved = codex_save_instance_configuration(
            instance_id.to_string(),
            None,
            None,
            None,
            None,
            Some(Some(CodexInstanceModelRouting::default())),
            None,
            None,
            None,
            None,
            Some(true),
            Some(true),
            Some(700_000),
            None,
            false,
            test_experimental_models(),
            None,
        )
        .await
        .expect("save instance configuration");

        let content =
            std::fs::read_to_string(profile_dir.join("config.toml")).expect("read saved config");
        assert!(content.contains("model_context_window = 700000"));
        // 统一口径：写了上下文窗口就必须同时写 90% 的压缩阈值。
        assert!(content.contains("model_auto_compact_token_limit = 630000"));
        assert_eq!(
            saved.quick_config.detected_model_context_window,
            Some(700_000)
        );
        assert_eq!(
            saved.quick_config.detected_auto_compact_token_limit,
            Some(630_000)
        );
    }

    #[tokio::test]
    async fn instance_configuration_save_catalog_only_context_preservation() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-catalog-context-preservation");
        let instance_id = "catalog-context-preservation-instance";
        let profile_dir =
            save_instance_with_quick_config(&env, instance_id, Some(750_000), Some(640_000));

        let saved = codex_save_instance_model_catalog(
            instance_id.to_string(),
            true,
            test_experimental_models(),
            None,
        )
        .await
        .expect("save instance model catalog");

        let content =
            std::fs::read_to_string(profile_dir.join("config.toml")).expect("read saved config");
        assert!(content.contains("model_context_window = 750000"));
        assert!(content.contains("model_auto_compact_token_limit = 640000"));
        assert_eq!(saved.detected_model_context_window, Some(750_000));
        assert_eq!(saved.detected_auto_compact_token_limit, Some(640_000));
    }

    #[tokio::test]
    async fn instance_configuration_save_keeps_default_context_unset() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-configuration-default-context");
        let instance_id = "default-context-instance";
        let profile_dir = save_instance_with_quick_config(&env, instance_id, None, None);

        let saved = codex_save_instance_configuration(
            instance_id.to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            test_experimental_models(),
            None,
        )
        .await
        .expect("save instance configuration");

        let content =
            std::fs::read_to_string(profile_dir.join("config.toml")).expect("read saved config");
        assert!(!content.contains("model_context_window"));
        assert!(!content.contains("model_auto_compact_token_limit"));
        assert_eq!(saved.quick_config.detected_model_context_window, None);
        assert_eq!(saved.quick_config.detected_auto_compact_token_limit, None);
    }

    #[tokio::test]
    async fn failed_instance_configuration_save_restores_context_window_settings() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = TestDataDirGuard::new("cockpit-codex-configuration-context-rollback");
        let instance_id = "context-rollback-instance";
        let profile_dir =
            save_instance_with_quick_config(&env, instance_id, Some(516_000), Some(460_000));

        let error = codex_save_instance_configuration(
            instance_id.to_string(),
            None,
            None,
            None,
            // 绑定一个不存在的账号：应用绑定时失败，用于验证上下文设置回滚。
            Some(Some("missing-bind-account".to_string())),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            Some(700_000),
            None,
            true,
            test_experimental_models(),
            None,
        )
        .await
        .expect_err("绑定不存在的账号应当保存失败");

        let content = std::fs::read_to_string(profile_dir.join("config.toml"))
            .expect("read rolled back config");
        assert!(content.contains("model_context_window = 516000"));
        assert!(content.contains("model_auto_compact_token_limit = 460000"));
    }
}
