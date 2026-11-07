#[test]
fn local_access_test_data_guards_keep_one_shared_lock_through_cleanup() {
    let sources = [
        (include_str!("codex_local_access_tests_key_recovery.rs"), 10),
        (
            include_str!("codex_local_access_tests_service_transitions.rs"),
            1,
        ),
        (
            include_str!("codex_local_access_tests_inbound_key_recovery.rs"),
            3,
        ),
        (
            include_str!("codex_local_access_tests_account_references.rs"),
            1,
        ),
    ];
    for (source, expected) in sources {
        let (checked, violations) = crate::modules::test_support::env_guard_order_violations(
            source,
            "LocalAccessTestDataGuard::new(",
        );
        assert_eq!(
            checked, expected,
            "all fixture constructors must be audited"
        );
        assert!(
            violations.is_empty(),
            "unprotected or double-locked fixtures: {violations:?}"
        );
    }
}

#[test]
fn local_access_guard_audit_accepts_owned_cache_lock_and_rejects_nested_lock() {
    let source = r#"
fn owned_cache_lock() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _env = LocalAccessTestDataGuard::new("cache");
}

fn nested_non_reentrant_lock() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let _env = LocalAccessTestDataGuard::new("nested");
}
fn early_cache_release() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _env = LocalAccessTestDataGuard::new("early");
    drop(shared_state);
}
"#;
    let (checked, violations) = crate::modules::test_support::env_guard_order_violations(
        source,
        "LocalAccessTestDataGuard::new(",
    );
    assert_eq!(checked, 3);
    assert_eq!(violations.len(), 2);
    assert!(violations
        .iter()
        .all(|violation| !violation.starts_with("owned_cache_lock:")));
}

#[test]
fn key_recovery_shared_state_fixtures_keep_one_lock_through_cleanup() {
    let cases = [
        (
            include_str!("codex_local_access_tests_key_recovery.rs"),
            "key_recovery_test_ticket(",
            12,
        ),
        (
            include_str!("codex_local_access_tests_service_transitions.rs"),
            "super::gateway_lifecycle_lock()",
            1,
        ),
        (
            include_str!("codex_local_access_tests_service_transitions.rs"),
            "super::local_access_enable_transition_lock()",
            1,
        ),
    ];
    for (source, operation, expected) in cases {
        let (checked, violations) =
            crate::modules::test_support::env_guard_order_violations(source, operation);
        assert_eq!(
            checked, expected,
            "all shared-state fixtures must be audited"
        );
        assert!(
            violations.is_empty(),
            "unprotected or double-locked shared state: {violations:?}"
        );
    }
}
