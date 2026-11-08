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
