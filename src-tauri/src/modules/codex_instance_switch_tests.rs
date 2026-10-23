use super::*;

struct Fixture {
    _lock: std::sync::MutexGuard<'static, ()>,
    root: PathBuf,
    previous: [Option<std::ffi::OsString>; 2],
}

impl Fixture {
    fn new() -> Self {
        let lock = modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("codex-switch-binding-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let keys = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"];
        let previous = keys.map(std::env::var_os);
        for key in keys {
            std::env::set_var(key, &root);
        }
        Self {
            _lock: lock,
            root,
            previous,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (key, previous) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
            .into_iter()
            .zip(&self.previous)
        {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn switch_binding_is_persisted_without_changing_launch_preferences() {
    let _fixture = Fixture::new();
    let mut store = InstanceStore::new();
    store.default_settings.bind_account_id = Some("previous".into());
    store.default_settings.extra_args = "--some-client-option".into();
    store.default_settings.launch_mode = InstanceLaunchMode::Cli;
    save_instance_store(&store).unwrap();
    bind_default_account_for_switch("next").unwrap();
    let saved = load_default_settings().unwrap();
    assert_eq!(saved.bind_account_id.as_deref(), Some("next"));
    assert!(!saved.follow_local_account);
    assert_eq!(saved.extra_args, "--some-client-option");
    assert_eq!(saved.launch_mode, InstanceLaunchMode::Cli);
}

#[test]
fn inaccessible_instance_store_fails_switch_binding_without_claiming_success() {
    let fixture = Fixture::new();
    let path = fixture.root.join(CODEX_INSTANCES_FILE);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("preserve"), "fixture").unwrap();
    assert_eq!(
        bind_default_account_for_switch("next").unwrap_err(),
        "CODEX_SWITCH_BINDING_FAILED"
    );
    assert_eq!(
        fs::read_to_string(path.join("preserve")).unwrap(),
        "fixture"
    );
}

#[test]
fn api_service_activation_uses_committed_binding_instead_of_previous_account() {
    let _fixture = Fixture::new();
    for (previous, follow_local) in [
        (Some("previous-oauth"), false),
        (Some("__provider_gateway__:previous-provider"), false),
        (None, true),
        (None, false),
        (Some(CODEX_API_SERVICE_BIND_ACCOUNT_ID), false),
    ] {
        let mut store = InstanceStore::new();
        store.default_settings.bind_account_id = previous.map(str::to_owned);
        store.default_settings.follow_local_account = follow_local;
        store.default_settings.extra_args = "--existing-option".into();
        store.default_settings.launch_mode = InstanceLaunchMode::Cli;
        save_instance_store(&store).unwrap();

        let expected = bind_default_api_service_for_launch().unwrap();
        let saved = load_default_settings().unwrap();
        assert_eq!(expected, CODEX_API_SERVICE_BIND_ACCOUNT_ID);
        assert_eq!(saved.bind_account_id.as_deref(), Some(expected));
        assert!(!saved.follow_local_account);
        assert_eq!(saved.extra_args, "--existing-option");
        assert_eq!(saved.launch_mode, InstanceLaunchMode::Cli);
        verify_prepared_launch_binding(Some(expected), saved.bind_account_id.as_deref()).unwrap();
    }
}

#[test]
fn api_service_binding_write_failure_does_not_prepare_a_launch_identity() {
    let fixture = Fixture::new();
    let path = fixture.root.join(CODEX_INSTANCES_FILE);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("preserve"), "fixture").unwrap();
    assert_eq!(
        bind_default_api_service_for_launch().unwrap_err(),
        "CODEX_SWITCH_BINDING_FAILED"
    );
    assert_eq!(
        fs::read_to_string(path.join("preserve")).unwrap(),
        "fixture"
    );
}

#[test]
fn api_service_launch_still_rejects_binding_changed_after_preparation() {
    let _fixture = Fixture::new();
    let expected = bind_default_api_service_for_launch().unwrap();
    bind_default_account_for_switch("another-oauth").unwrap();
    let saved = load_default_settings().unwrap();
    assert_eq!(
        verify_prepared_launch_binding(Some(expected), saved.bind_account_id.as_deref()).unwrap_err(),
        "CODEX_SWITCH_BINDING_FAILED"
    );
}

#[test]
fn prepared_profile_rejects_changed_or_missing_binding_before_launch() {
    assert!(verify_prepared_launch_binding(Some("next"), Some("next")).is_ok());
    for actual in [None, Some("previous"), Some("__provider_gateway__:next")] {
        assert_eq!(
            verify_prepared_launch_binding(Some("next"), actual).unwrap_err(),
            "CODEX_SWITCH_BINDING_FAILED"
        );
    }
    assert!(verify_prepared_launch_binding(None, Some("regular-start")).is_ok());
}

#[test]
fn old_reauth_status_does_not_hide_a_current_launch_setup_failure() {
    let _fixture = Fixture::new();
    let mut account = crate::models::codex::CodexAccount::new(
        "switch-setup-error".into(), "fixture@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "fixture-access".into(),
            refresh_token: Some("fixture-refresh".into()), id_token: "fixture-id".into(),
        },
    );
    account.requires_reauth = true;
    account.reauth_reason = Some("refresh_token_expired".into());
    modules::codex_account::save_account(&account).unwrap();
    for code in ["CODEX_SWITCH_BINDING_FAILED", "CODEX_PROCESS_SCAN_FAILED", "PROXY_ENTRY_PORT_UNAVAILABLE"] {
        assert_eq!(modules::codex_account::format_account_switch_error(&account.id, code.into()), code);
    }
    assert!(modules::codex_account::load_account(&account.id).unwrap().requires_reauth);
}
