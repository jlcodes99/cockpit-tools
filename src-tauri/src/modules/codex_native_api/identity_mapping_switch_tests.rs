use super::*;
use serde_json::json;

fn request() -> (HeaderMap, Value) {
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("cache-session"));
    headers.insert("thread-id", HeaderValue::from_static("header-thread"));
    headers.insert(
        "x-client-request-id",
        HeaderValue::from_static("header-thread"),
    );
    headers.insert(
        "x-codex-parent-thread-id",
        HeaderValue::from_static("parent"),
    );
    let metadata = json!({"session_id":"canonical-session","thread_id":"canonical-thread",
        "parent_thread_id":"parent","window_id":"canonical-thread:7","turn_id":"turn"});
    headers.insert(
        "x-codex-turn-metadata",
        HeaderValue::from_str(&metadata.to_string()).unwrap(),
    );
    let body = json!({"model":"m","input":[],"prompt_cache_key":"cache-session",
        "client_metadata":{"session_id":"flat-session","thread_id":"flat-thread",
            "x-codex-turn-metadata":metadata.to_string()}});
    (headers, body)
}

#[test]
fn disabled_mapping_preserves_original_identity_fields_without_creating_mappings() {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    store.set_session_identity_mapping_enabled(false);
    let (mut headers, mut body) = request();
    let expected = (headers.clone(), body.clone());
    let context = store.inspect("caller", &headers, &body).unwrap();
    store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!((headers, body), expected);
    let count: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM native_identity_v1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let (mut headers, mut body) = (HeaderMap::new(), json!({"model":"m","input":[]}));
    let context = store.inspect("caller", &headers, &body).unwrap();
    store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert!(headers.is_empty());
    assert!(body.get("client_metadata").is_none());
}

#[test]
fn mapping_policy_is_snapshotted_and_reenabling_reuses_persisted_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    let (mut headers, mut body) = request();
    let mapped_context = store.inspect("caller", &headers, &body).unwrap();
    store.set_session_identity_mapping_enabled(false);
    store
        .rewrite(&mapped_context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    let mapped = headers.clone();
    assert_ne!(headers["session-id"], "cache-session");
    assert_ne!(headers["thread-id"], "header-thread");

    let (mut headers, mut body) = request();
    let context = store.inspect("caller", &headers, &body).unwrap();
    store.set_session_identity_mapping_enabled(true);
    let binding = store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!((headers.clone(), body.clone()), request());
    let mut continuation = store.inspect("caller", &headers, &body).unwrap();
    continuation.retain_binding_generation(&binding);
    store
        .rewrite(&continuation, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!((headers, body), request());

    drop(store);
    let reopened = IdentityStore::open(path).unwrap();
    let (mut headers, mut body) = request();
    let context = reopened.inspect("caller", &headers, &body).unwrap();
    reopened
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers, mapped);
}

#[test]
fn disabled_mapping_keeps_upstream_state_owner_and_caller_checks() {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    store.set_session_identity_mapping_enabled(false);
    let (mut headers, mut body) = request();
    let context = store.inspect("caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    store
        .observe_event(
            &binding,
            &json!({"type":"response.completed",
        "response":{"id":"response-owned","status":"completed","output":[]}}),
        )
        .unwrap();
    body["previous_response_id"] = json!("response-owned");
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(
        store
            .rewrite(&context, "other", "other-owner", &mut headers, &mut body)
            .unwrap_err()
            .code,
        "upstream_state_owner_changed"
    );
    assert_eq!(
        store
            .inspect("other-caller", &headers, &body)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["session-id"], "cache-session");
    assert_eq!(headers["thread-id"], "header-thread");
}
