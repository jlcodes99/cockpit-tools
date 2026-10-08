#[tokio::test]
async fn provider_gateway_ready_failure_preserves_profile_without_publishing() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("provider-ready-failure");
    let profile = make_temp_dir("provider-ready-profile");
    let config_path = super::profile_config_path(&profile);
    let auth_path = super::profile_auth_path(&profile);
    fs::write(&config_path, "model_provider = \"openai\"\n").unwrap();
    fs::write(&auth_path, "original oauth auth").unwrap();
    let published = std::cell::Cell::new(false);
    let cleaned = std::cell::Cell::new(false);
    let result = super::activate_provider_gateway_profile(
        &profile,
        "synthetic-provider-key",
        || async { Err::<(), _>("sidecar ready failed".to_string()) },
        || async {
            published.set(true);
            Ok(())
        },
        |_| async {
            cleaned.set(true);
        },
    )
    .await;
    assert_eq!(result, Err("sidecar ready failed".to_string()));
    assert!(!published.get());
    assert!(
        !cleaned.get(),
        "failed spawn owns cleanup before returning its error"
    );
    assert_eq!(
        fs::read_to_string(config_path).unwrap(),
        "model_provider = \"openai\"\n"
    );
    assert_eq!(
        fs::read_to_string(auth_path).unwrap(),
        "original oauth auth"
    );
    assert!(super::load_takeover_backups().unwrap().profiles.is_empty());
    let _ = fs::remove_dir_all(profile);
}

#[tokio::test]
async fn provider_gateway_publish_failure_restores_profile_and_releases_new_listener() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("provider-publish-failure");
    let profile = make_temp_dir("provider-publish-profile");
    let other_profile = make_temp_dir("provider-unrelated-profile");
    let config_path = super::profile_config_path(&profile);
    let auth_path = super::profile_auth_path(&profile);
    let catalog_path = profile.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE);
    let cache_path = profile.join(CODEX_MODEL_CACHE_FILE);
    let api_key = "synthetic-provider-key";
    // An existing gateway takeover must be restored exactly, not rolled back to official login.
    let original_config = format!("model_provider = \"codex_local_access\"\n[model_providers.codex_local_access]\nexperimental_bearer_token = \"{api_key}\"\nbase_url = \"http://localhost:12345/v1\"\n");
    fs::write(&config_path, &original_config).unwrap();
    fs::write(&auth_path, "original bound OAuth auth").unwrap();
    fs::write(&catalog_path, "original catalog").unwrap();
    fs::write(
        super::profile_config_path(&other_profile),
        "unrelated config",
    )
    .unwrap();
    super::save_profile_takeover_backup(&other_profile, "other-key").unwrap();
    let other_backup_before =
        serde_json::to_value(super::load_takeover_backups().unwrap()).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let result = super::activate_provider_gateway_profile(
        &profile,
        api_key,
        || async { Ok(listener) },
        || async {
            super::save_profile_takeover_backup(&profile, api_key)?;
            fs::write(
                &config_path,
                format!("failed new endpoint localhost:{port}"),
            )
            .unwrap();
            fs::write(&auth_path, "partial auth").unwrap();
            fs::write(&catalog_path, "partial catalog").unwrap();
            fs::write(&cache_path, "new file from failed publication").unwrap();
            Err("model catalog write failed".to_string())
        },
        |listener| async move {
            drop(listener);
        },
    )
    .await;
    assert_eq!(result.unwrap_err(), "model catalog write failed");
    assert!(super::is_local_access_port_bindable("127.0.0.1", port).unwrap());
    assert_eq!(fs::read_to_string(config_path).unwrap(), original_config);
    assert_eq!(
        fs::read_to_string(auth_path).unwrap(),
        "original bound OAuth auth"
    );
    assert_eq!(
        fs::read_to_string(catalog_path).unwrap(),
        "original catalog"
    );
    assert!(!cache_path.exists());
    assert_eq!(
        serde_json::to_value(super::load_takeover_backups().unwrap()).unwrap(),
        other_backup_before
    );
    assert_eq!(
        fs::read_to_string(super::profile_config_path(&other_profile)).unwrap(),
        "unrelated config"
    );
    let _ = fs::remove_dir_all(profile);
    let _ = fs::remove_dir_all(other_profile);
}

#[tokio::test]
async fn provider_gateway_publishes_only_after_ready_and_keeps_listener_on_success() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("provider-ready-before-publish");
    let profile = make_temp_dir("provider-ready-order");
    let config_path = super::profile_config_path(&profile);
    fs::write(&config_path, "original config").unwrap();
    let ready = std::cell::Cell::new(false);
    let cleaned = std::cell::Cell::new(false);
    let listener = super::activate_provider_gateway_profile(
        &profile,
        "synthetic-provider-key",
        || async {
            tokio::task::yield_now().await;
            assert_eq!(fs::read_to_string(&config_path).unwrap(), "original config");
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            ready.set(true);
            Ok(listener)
        },
        || async {
            assert!(ready.get());
            fs::write(&config_path, "ready endpoint").unwrap();
            Ok(())
        },
        |listener| async {
            drop(listener);
            cleaned.set(true);
        },
    )
    .await
    .unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(!super::is_local_access_port_bindable("127.0.0.1", port).unwrap());
    assert!(!cleaned.get());
    assert_eq!(fs::read_to_string(config_path).unwrap(), "ready endpoint");
    drop(listener);
    let _ = fs::remove_dir_all(profile);
}
