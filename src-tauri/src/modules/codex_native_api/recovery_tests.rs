//! Historical journal compatibility and ordinary terminal validation.
use super::super::identity::IdentityStore;
use axum::http::HeaderMap;
use serde_json::json;
use std::path::PathBuf;
fn fixture() -> (tempfile::TempDir, PathBuf, IdentityStore) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    (temp, path, store)
}
fn invalid_checkpoint_events() -> Vec<serde_json::Value> {
    vec![
        json!({"type":"response.completed","response":{"id":"bad","status":"completed","error":{"code":"server_error"},"output":[]}}),
        json!({"type":"response.done","response":{"id":"bad","status":"completed","error":false,"output":[]}}),
        json!({"id":"bad","status":"completed","error":{"code":"server_error"},"output":[]}),
        json!({"type":"response.completed","error":{"code":"server_error"},"response":{"id":"bad","status":"completed","error":null,"output":[]}}),
        json!({"type":"response.completed","response":{"id":"bad","status":"in_progress","error":null,"output":[]}}),
        json!({"type":"response.done","response":{"id":"bad","error":null,"output":[]}}),
        json!({"type":"response.created","response":{"id":"bad","status":"completed","error":null,"output":[]}}),
        json!({"type":"response.failed","response":{"id":"bad","status":"completed","error":null,"output":[]}}),
        json!({"type":"response.completed","response":{"id":"   ","status":"completed","error":null,"output":[]}}),
    ]
}
#[test]
fn normal_json_and_sse_checkpoint_requires_completed_status_id_and_null_error() {
    for sse in [false, true] {
        let mut cases = invalid_checkpoint_events()
            .into_iter()
            .map(|event| (event, false))
            .collect::<Vec<_>>();
        for kind in [
            None,
            Some("response"),
            Some("response.completed"),
            Some("response.done"),
        ] {
            let response = json!({"id":"valid","status":"completed","error":null,"output":[]});
            cases.push((
                kind.map_or(
                    response.clone(),
                    |kind| json!({"type":kind,"response":response}),
                ),
                true,
            ));
        }
        for (event, expected) in cases {
            let (_temp, path, store) = fixture();
            let mut input =
                json!({"client_metadata":{"session_id":"root","thread_id":"child"},"input":[]});
            let mut headers = HeaderMap::new();
            let context = store.inspect("caller", &headers, &input).unwrap();
            let binding = store
                .rewrite(&context, "account", "owner", &mut headers, &mut input)
                .unwrap();
            if sse {
                let mut decoder = super::super::events::SseDecoder::new(4096);
                if let Ok(frames) = decoder.push_frames(format!("data: {event}\n\n").as_bytes()) {
                    for frame in frames {
                        if let Some(value) = frame.value {
                            store.observe_event(&binding, &value).unwrap();
                        }
                    }
                }
            } else {
                store.observe_event(&binding, &event).unwrap();
            }
            drop(store);
            let store = IdentityStore::open(path.clone()).unwrap();
            let connection = rusqlite::Connection::open(&path).unwrap();
            let checkpoints: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(checkpoints, i64::from(expected), "sse={sse}, event={event}");
            let response_id = event.get("response").unwrap_or(&event)["id"].clone();
            let follow = json!({"previous_response_id":response_id,"input":[]});
            assert_eq!(
                store.inspect("caller", &HeaderMap::new(), &follow).is_ok(),
                expected,
                "sse={sse}, event={event}"
            );
        }
    }
}

#[test]
fn retired_manual_controls_are_not_registered_or_exposed_by_the_bridge() {
    let commands = include_str!("../../commands/codex_local_access_commands.rs");
    let app = include_str!("../../lib.rs");
    for name in [
        "codex_local_access_legacy_recovery_get",
        "codex_local_access_legacy_recovery_arm",
        "codex_local_access_legacy_recovery_cancel",
    ] {
        assert!(!commands.contains(name));
        assert!(!app.contains(name));
    }
    let bridge = include_str!("continuity_bridge/protocol.rs");
    assert!(!bridge.contains("pub recovery:"));
}

#[test]
fn historical_completion_rows_remain_data_only_and_roundtrip_with_future_fields() {
    let record = json!({"id":"old-completion","caller":"caller","thread":"thread","account":"account","owner":"owner","completed_at":42,"future":true});
    let completion: super::Completion = serde_json::from_value(record).unwrap();
    assert_eq!(completion.completed_at, 42);
    assert_eq!(serde_json::to_value(&completion).unwrap()["owner"], "owner");
}
