#[test]
fn unprepared_account_proxy_never_falls_back_to_raw_sidecar_url() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let mut account = super::CodexAccount::new(
        "unprepared-authenticated-sidecar-proxy".into(),
        "test@example.com".into(),
        crate::models::codex::CodexTokens {
            access_token: "access".into(),
            id_token: "id".into(),
            refresh_token: Some("refresh".into()),
        },
    );
    account.egress_proxy_url = Some("http://user:secret@proxy.example:8080".into());
    assert!(super::sidecar_proxy_url_for_account(&account, None).is_err());
    account.egress_proxy_url = Some("http://proxy.example:8080".into());
    assert_eq!(
        super::sidecar_proxy_url_for_account(&account, None).unwrap_err(),
        "PROXY_RUNTIME_NOT_READY"
    );
    account.egress_proxy_url = None;
    assert_eq!(
        super::sidecar_proxy_url_for_account(&account, Some("http://global.example:8080"))
            .unwrap_err(),
        "UNIFIED_PROXY_LOADING"
    );
    account.egress_proxy_disabled = true;
    assert_eq!(
        super::sidecar_proxy_url_for_account(&account, Some("http://global.example:8080"))
            .unwrap()
            .as_deref(),
        Some("direct"),
        "explicit opt-out must not inherit global or shared proxy settings"
    );
    account.egress_proxy_disabled = false;
    shared_state.prepare_disabled();
    assert_eq!(
        super::sidecar_proxy_url_for_account(&account, Some("http://global.example:8080"))
            .unwrap()
            .as_deref(),
        Some("http://global.example:8080")
    );
}
