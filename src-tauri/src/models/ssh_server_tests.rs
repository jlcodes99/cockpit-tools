use super::*;
use serde_json::json;

#[test]
fn legacy_selection_migrates_and_explicit_empty_multiple_selection_wins() {
    let old: SshServerStore = serde_json::from_value(json!({
        "version":"1", "selected_server_id":"old-host", "servers":[]
    }))
    .unwrap();
    let saved = serde_json::to_value(old).unwrap();
    assert_eq!(saved["selected_server_ids"], json!(["old-host"]));
    assert!(saved.get("selected_server_id").is_none());
    let cleared: SshServerStore = serde_json::from_value(json!({
        "version":"2", "selected_server_ids":[], "selected_server_id":"old-host", "servers":[]
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(cleared).unwrap()["selected_server_ids"],
        json!([])
    );
}

#[test]
fn uploaded_credentials_with_a_failed_refresh_require_another_sync() {
    let mut result = SshCodexSyncResult {
        server_id: "fixture".into(),
        server_name: "Fixture".into(),
        account_id: "codex_local_access_runtime".into(),
        account_email: "API Service".into(),
        token_generation: 0,
        bundle_hash: "fixture".into(),
        verified: true,
        error: Some("ssh_connection_refresh_failed: desktop_startup_timeout".into()),
        synced_at: 0,
    };
    assert!(!result.succeeded());
    result.error = None;
    assert!(result.succeeded());
    result.verified = false;
    assert!(!result.succeeded());
}
