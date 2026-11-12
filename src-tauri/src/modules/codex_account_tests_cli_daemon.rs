#[test]
fn daemon_notice_requires_a_successful_auth_commit_even_if_config_later_fails() {
    use std::cell::Cell;
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let account = CodexAccount::new(
        "daemon-notice-test".to_string(),
        "daemon@example.com".to_string(),
        CodexTokens {
            id_token: "id.token".to_string(),
            access_token: "access.token".to_string(),
            refresh_token: Some("refresh.token".to_string()),
        },
    );
    for blocked in [None, Some("auth.json"), Some("config.toml")] {
        let dir = make_temp_dir("codex-daemon-commit");
        if let Some(blocked) = blocked {
            fs::create_dir(dir.join(blocked)).unwrap();
        }
        let notices = Cell::new(0);
        let result = super::write_auth_file_to_dir_with_after_commit(&dir, &account, || {
            assert!(dir.join("auth.json").is_file());
            notices.set(notices.get() + 1);
        });
        assert_eq!(notices.get(), usize::from(blocked != Some("auth.json")));
        assert_eq!(result.is_ok(), blocked.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn daemon_notice_does_not_repeat_for_an_unchanged_auth_bundle() {
    use std::cell::Cell;
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = make_temp_dir("codex-daemon-unchanged");
    let mut account = CodexAccount::new(
        "daemon-unchanged-test".to_string(),
        "daemon@example.com".to_string(),
        CodexTokens {
            id_token: "id.token".to_string(),
            access_token: "access.token".to_string(),
            refresh_token: Some("refresh.token".to_string()),
        },
    );
    let notices = Cell::new(0);
    for _ in 0..2 {
        super::write_auth_file_to_dir_with_after_commit(&dir, &account, || {
            notices.set(notices.get() + 1);
        })
        .unwrap();
    }
    assert_eq!(notices.get(), 1);
    account.tokens.access_token = "next.access.token".to_string();
    super::write_auth_file_to_dir_with_after_commit(&dir, &account, || {
        notices.set(notices.get() + 1);
    })
    .unwrap();
    assert_eq!(notices.get(), 2);
    // Access-token-only accounts use the official personal_access_token shape,
    // not the tokens object. Their credential changes need the same notice.
    account.tokens.id_token.clear();
    account.tokens.refresh_token = None;
    for token in ["pat.first", "pat.first", "pat.next"] {
        account.tokens.access_token = token.into();
        super::write_auth_file_to_dir_with_after_commit(&dir, &account, || {
            notices.set(notices.get() + 1);
        })
        .unwrap();
    }
    assert_eq!(notices.get(), 4);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn switching_supplier_with_the_same_key_notifies_after_the_bundle_is_written() {
    use std::cell::Cell;
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = make_temp_dir("codex-daemon-supplier-switch");
    let make_account = |name: &str| {
        let mut account = CodexAccount::new_api_key(
            name.into(),
            format!("{name}@example.com"),
            "test-shared-api-key".into(),
            CodexApiProviderMode::Custom,
            Some(format!("https://{name}.example.com/v1")),
            Some(name.into()),
            Some(name.into()),
            vec![format!("{name}-model")],
        );
        account.api_instance_access_mode = Some("direct".to_string());
        account.api_sync_model_catalog_to_codex = true;
        account
    };
    let first = make_account("first");
    let second = make_account("second");
    super::write_prepared_account_bundle_to_dir(&dir, &first).unwrap();
    let notices = Cell::new(0);
    for _ in 0..2 {
        super::write_bundle_with_cli_daemon_notice(
            &dir,
            |credentials_changed| {
                super::write_prepared_account_bundle(&dir, &second, credentials_changed)?;
                assert!(!credentials_changed.get());
                Ok(())
            },
            |complete| {
                assert!(complete);
                assert_eq!(
                    super::configured_provider_base_url(&dir).as_deref(),
                    Some("https://second.example.com/v1")
                );
                assert_eq!(
                    super::read_managed_projection_account_id_from_dir(&dir).as_deref(),
                    Some("second")
                );
                let catalog =
                    fs::read_to_string(dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap();
                assert!(catalog.contains("second-model"));
                assert!(!catalog.contains("first-model"));
                notices.set(notices.get() + 1);
            },
        )
        .unwrap();
    }
    assert_eq!(notices.get(), 1);
    fs::remove_dir_all(dir).unwrap();
}
