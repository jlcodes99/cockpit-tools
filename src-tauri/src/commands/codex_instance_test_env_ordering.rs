// Audit the actual fixture call sites, without mutating this test process's env.
// The common pattern declares the lock before the data guard, so Rust drops the
// data guard (including env restoration and directory cleanup) before the lock.
fn audit_instance_test_guard_lock_order(source: &str) -> (usize, Vec<String>) {
    crate::modules::test_support::env_guard_order_violations(source, "TestDataDirGuard::new(")
}

#[test]
fn instance_test_data_guards_hold_shared_lock_through_cleanup() {
    let (checked, violations) =
        audit_instance_test_guard_lock_order(include_str!("codex_instance.rs"));
    assert_eq!(
        checked, 11,
        "audit must cover every current data guard fixture"
    );
    assert!(
        violations.is_empty(),
        "unprotected instance fixtures: {violations:?}"
    );
}

#[test]
fn instance_test_data_guard_audit_rejects_unlocked_and_early_release() {
    let source = r#"
async fn unlocked() {
    let env = TestDataDirGuard::new("unlocked");
}
async fn released_before_create() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    drop(_lock);
    let env = TestDataDirGuard::new("released");
}
async fn released_before_cleanup() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let env = TestDataDirGuard::new("released");
    drop(_lock);
}
"#;
    let (checked, violations) = audit_instance_test_guard_lock_order(source);
    assert_eq!(checked, 3);
    assert_eq!(violations.len(), 3, "each unsafe lifetime must be rejected");
}
