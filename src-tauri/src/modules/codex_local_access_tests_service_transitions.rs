#[test]
fn disable_restores_profiles_even_when_gateway_shutdown_fails() {
    let mut restored = false;
    let result = super::finish_local_access_disable(Err("port still occupied".to_string()), || {
        restored = true;
        Ok(())
    });
    assert!(restored);
    assert_eq!(result, Err("port still occupied".to_string()));
}

#[test]
fn disable_preserves_both_shutdown_and_profile_restore_failures() {
    let result = super::finish_local_access_disable(Err("port still occupied".to_string()), || {
        Err("profile restore failed".to_string())
    });
    let error = result.expect_err("both operations failed");
    assert!(error.contains("port still occupied"));
    assert!(error.contains("profile restore failed"));
}

#[test]
fn disable_reports_profile_restore_failure_after_successful_shutdown() {
    assert_eq!(
        super::finish_local_access_disable(Ok(()), || Err("profile restore failed".to_string())),
        Err("profile restore failed".to_string()),
    );
    assert_eq!(super::finish_local_access_disable(Ok(()), || Ok(())), Ok(()));
}
