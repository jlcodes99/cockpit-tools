//! Scope handling that only this tree carries.
//!
//! `oauth_tests.rs` is compiled by both `cockpit-core` and `cockpit-tools`, so it
//! may only use API the two copies share. `scope_lacks_aicode`,
//! `TokenResponse::scope` and `TokenData::{lacks_aicode_scope, granted_scopes}`
//! exist only in the desktop tree, so the cases that need them live here instead
//! of breaking the core test build.

use super::*;

#[test]
fn detects_missing_aicode_scope_and_persists_granted_scopes() {
    assert!(!scope_lacks_aicode(None));
    assert!(!scope_lacks_aicode(Some("   ")));
    assert!(!scope_lacks_aicode(Some(
        "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/aicode"
    )));
    assert!(scope_lacks_aicode(Some(
        "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email"
    )));

    let current = crate::models::TokenData::new(
        "old-access".into(),
        "old-refresh".into(),
        60,
        Some("fixture@example.com".into()),
        None,
        None,
    );
    let refreshed = build_refreshed_token(
        &current,
        TokenResponse {
            access_token: "new-access".into(),
            expires_in: 3600,
            token_type: "Bearer".into(),
            refresh_token: None,
            id_token: None,
            scope: Some("openid https://www.googleapis.com/auth/cloud-platform".into()),
            oauth_client_key: Some(DEFAULT_OAUTH_CLIENT_KEY.into()),
        },
    );
    assert!(refreshed.lacks_aicode_scope());
    assert_eq!(
        refreshed.granted_scopes.as_deref(),
        Some("openid https://www.googleapis.com/auth/cloud-platform")
    );
}
