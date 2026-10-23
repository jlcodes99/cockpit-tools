#[test]
fn native_host_identity_fixture_survives_environment_rotation_and_keeps_validation() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    struct RestoreDataDirectory(Option<std::ffi::OsString>);
    impl Drop for RestoreDataDirectory {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", previous);
            } else {
                std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
            }
        }
    }
    let _restore = RestoreDataDirectory(std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let old_global = tempfile::tempdir().unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", old_global.path());
        let host = native_test_routing_host();
        let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
        let mut request = native_test_route_request("isolated-installation");
        request.preferred_account_id = Some("pool-a".into());
        let first = native_select_route(&host, &caller, request.clone())
            .await
            .unwrap();
        let profile = first.installation_profile.clone().unwrap();
        drop(first);
        assert!(host.identity_fixture.path().is_file());
        assert!(!old_global
            .path()
            .join("codex_api_account_identity.sqlite")
            .exists());

        // This is the failing concurrent fixture lifecycle: the global path
        // changes and its prior directory disappears between route attempts.
        let new_global = tempfile::tempdir().unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", new_global.path());
        drop(old_global);
        let second = native_select_route(&host, &caller, request.clone())
            .await
            .unwrap();
        assert_eq!(second.installation_profile.as_ref(), Some(&profile));
        drop(second);

        let independent = native_test_routing_host();
        let other = native_select_route(&independent, &caller, request.clone())
            .await
            .unwrap();
        assert_ne!(other.installation_profile.as_ref(), Some(&profile));
        drop(other);
        assert_ne!(
            NativeHostBridge::default().identity_fixture.path(),
            host.identity_fixture.path()
        );

        // The fixture uses the production database validation, not a synthetic
        // installation ID or a success-only mock.
        let database = rusqlite::Connection::open(host.identity_fixture.path()).unwrap();
        database.pragma_update(None, "user_version", 99).unwrap();
        drop(database);
        let error = native_select_route(&host, &caller, request)
            .await
            .err()
            .unwrap();
        assert_eq!(
            (error.status, error.code),
            (503, "identity_profile_unavailable")
        );
        assert!(!new_global
            .path()
            .join("codex_api_account_identity.sqlite")
            .exists());
    });
}
