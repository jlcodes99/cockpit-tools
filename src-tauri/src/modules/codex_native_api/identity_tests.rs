use super::*;
use serde_json::json;

#[test]
fn legacy_session_header_is_stable_and_stays_caller_and_owner_scoped() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let project = |caller: &str, account: &str, owner: &str, session: &str| {
        let mut headers = HeaderMap::new();
        headers.insert("session_id", HeaderValue::from_str(session).unwrap());
        let mut body = json!({"input":"synthetic prompt", "prompt_cache_key":"cache-hint"});
        let context = store.inspect(caller, &headers, &body).unwrap();
        assert_eq!(context.session_id.as_deref(), Some(session));
        assert_eq!(context.thread_id.as_deref(), Some(session));
        store
            .rewrite(&context, account, owner, &mut headers, &mut body)
            .unwrap();
        assert!(!headers.contains_key("session_id"));
        assert_ne!(headers["session-id"], session);
        assert_eq!(
            headers["session-id"].to_str().unwrap(),
            body["client_metadata"]["session_id"].as_str().unwrap()
        );
        headers["session-id"].clone()
    };
    let first = project("caller", "account-a", "owner-a", "pi-session");
    assert_eq!(
        first,
        project("caller", "account-a", "owner-a", "pi-session")
    );
    assert_ne!(
        first,
        project("caller", "account-b", "owner-b", "pi-session")
    );
    assert_ne!(
        first,
        project("other-caller", "account-a", "owner-a", "pi-session")
    );
    assert_ne!(
        first,
        project("caller", "account-a", "owner-a", "other-session")
    );
}

#[test]
fn legacy_session_header_respects_metadata_and_canonical_header_precedence() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("session_id", HeaderValue::from_static("legacy-session"));
    headers.insert("session-id", HeaderValue::from_static("header-session"));
    for (body, expected) in [
        (json!({"input":"test"}), "header-session"),
        (
            json!({"client_metadata":{"session_id":"flat-session"}}),
            "flat-session",
        ),
        (
            json!({"client_metadata":{
                "session_id":"flat-session",
                "x-codex-turn-metadata":json!({"session_id":"canonical-session"}).to_string()
            }}),
            "canonical-session",
        ),
    ] {
        assert_eq!(
            store
                .inspect("caller", &headers, &body)
                .unwrap()
                .session_id
                .as_deref(),
            Some(expected)
        );
    }
    headers.insert("session-id", HeaderValue::from_static(""));
    assert_eq!(
        store
            .inspect("caller", &headers, &json!({}))
            .unwrap()
            .session_id
            .as_deref(),
        Some("legacy-session")
    );
}

#[test]
fn legacy_session_header_cannot_bypass_invalid_higher_priority_identity() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("session_id", HeaderValue::from_static("legacy-session"));
    headers.insert(
        "session-id",
        HeaderValue::from_str(&"x".repeat(MAX_ID_BYTES + 1)).unwrap(),
    );
    assert_eq!(
        store
            .inspect("caller", &headers, &json!({}))
            .unwrap_err()
            .code,
        "invalid_protocol_identity"
    );
    headers.remove("session-id");
    for metadata in [
        json!({"session_id":123}),
        json!({"x-codex-turn-metadata":json!({"session_id":123}).to_string()}),
    ] {
        assert_eq!(
            store
                .inspect("caller", &headers, &json!({"client_metadata":metadata}))
                .unwrap_err()
                .code,
            "invalid_protocol_identity"
        );
    }
}

#[test]
fn legacy_session_header_validates_only_when_selected_and_keeps_empty_policy() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    for invalid in [
        HeaderValue::from_str(&"x".repeat(MAX_ID_BYTES + 1)).unwrap(),
        HeaderValue::from_bytes(b"bad\tidentity").unwrap(),
        HeaderValue::from_bytes(b"\xff").unwrap(),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("session_id", invalid);
        assert_eq!(
            store
                .inspect("caller", &headers, &json!({}))
                .unwrap_err()
                .code,
            "invalid_protocol_identity"
        );
        headers.insert("session-id", HeaderValue::from_static("valid-session"));
        let mut body = json!({"input":"test"});
        let context = store.inspect("caller", &headers, &body).unwrap();
        assert_eq!(context.session_id.as_deref(), Some("valid-session"));
        store
            .rewrite(&context, "account", "owner", &mut headers, &mut body)
            .unwrap();
        assert!(!headers.contains_key("session_id"));
    }
    let mut headers = HeaderMap::new();
    headers.insert("session_id", HeaderValue::from_static(""));
    let first = store.inspect("caller", &headers, &json!({})).unwrap();
    let second = store.inspect("caller", &headers, &json!({})).unwrap();
    assert!(first.session_id.as_ref().is_some_and(|id| !id.is_empty()));
    assert_ne!(first.session_id, second.session_id);
}

struct Database {
    directory: PathBuf,
    path: PathBuf,
}

impl Database {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("cockpit-native-identity-{}", uuid::Uuid::now_v7()));
        Self {
            path: directory.join("identity.sqlite"),
            directory,
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn request(session: &str, thread: &str, parent: Option<&str>) -> (HeaderMap, Value) {
    let canonical = json!({
        "session_id": session, "thread_id": thread, "turn_id": format!("turn-{thread}"),
        "parent_thread_id": parent, "root_turn_id": "turn-root", "window_id": format!("{thread}:7"),
        "context_window_id": "context-1", "request_kind": "turn"
    });
    let raw = serde_json::to_string(&canonical).unwrap();
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("session-id", session.to_owned()),
        ("thread-id", thread.to_owned()),
        ("x-client-request-id", thread.to_owned()),
        ("x-codex-window-id", format!("{thread}:7")),
        ("x-codex-turn-metadata", raw.clone()),
    ] {
        headers.insert(name, HeaderValue::from_str(&value).unwrap());
    }
    if let Some(parent) = parent {
        headers.insert(
            "x-codex-parent-thread-id",
            HeaderValue::from_str(parent).unwrap(),
        );
    }
    let body = json!({
        "model": "test", "prompt_cache_key": session,
        "client_metadata": {
            "session_id": session, "thread_id": thread, "turn_id": format!("turn-{thread}"),
            "root_turn_id": "turn-root", "x-codex-parent-thread-id": parent,
            "x-codex-window-id": format!("{thread}:7"), "x-codex-turn-metadata": raw,
        },
        "input": [{"type":"message", "role":"user", "content":[{"type":"input_text","text":"root session child turn-root context-1"}]},
                  {"type":"function_call", "id":"tool-item", "call_id":"root", "name":"test", "arguments":"{\"thread_id\":\"root\"}"}]
    });
    (headers, body)
}

fn mapped(
    store: &IdentityStore,
    caller: &str,
    account: &str,
    owner: &str,
    session: &str,
    thread: &str,
    parent: Option<&str>,
) -> (HeaderMap, Value, Binding) {
    let (mut headers, mut body) = request(session, thread, parent);
    let context = store.inspect(caller, &headers, &body).unwrap();
    let binding = store
        .rewrite(&context, account, owner, &mut headers, &mut body)
        .unwrap();
    (headers, body, binding)
}

#[test]
fn hierarchy_preserves_shared_session_distinct_threads_and_references() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (root_headers, root, _) = mapped(&store, "caller", "a", "owner-a", "session", "root", None);
    let (child_headers, child, _) = mapped(
        &store,
        "caller",
        "a",
        "owner-a",
        "session",
        "child",
        Some("root"),
    );
    let (grand_headers, grand, _) = mapped(
        &store,
        "caller",
        "a",
        "owner-a",
        "session",
        "grandchild",
        Some("child"),
    );
    assert_eq!(root_headers["session-id"], child_headers["session-id"]);
    assert_eq!(child_headers["session-id"], grand_headers["session-id"]);
    assert_eq!(root["prompt_cache_key"], child["prompt_cache_key"]);
    assert_eq!(child["prompt_cache_key"], grand["prompt_cache_key"]);
    assert_ne!(root_headers["thread-id"], child_headers["thread-id"]);
    assert_ne!(child_headers["thread-id"], grand_headers["thread-id"]);
    assert_eq!(
        root_headers["thread-id"],
        child_headers["x-codex-parent-thread-id"]
    );
    assert_eq!(
        child_headers["thread-id"],
        grand_headers["x-codex-parent-thread-id"]
    );
    for (headers, body) in [
        (&root_headers, &root),
        (&child_headers, &child),
        (&grand_headers, &grand),
    ] {
        let canonical: Value = serde_json::from_str(
            body["client_metadata"]["x-codex-turn-metadata"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(headers["thread-id"], headers["x-client-request-id"]);
        assert_eq!(
            canonical["thread_id"].as_str().unwrap(),
            headers["thread-id"].to_str().unwrap()
        );
        assert_eq!(
            canonical["window_id"],
            format!("{}:7", headers["thread-id"].to_str().unwrap())
        );
        assert_ne!(canonical["context_window_id"], canonical["window_id"]);
        assert_eq!(body["input"], request("session", "root", None).1["input"]);
        assert_eq!(
            uuid::Uuid::parse_str(headers["thread-id"].to_str().unwrap())
                .unwrap()
                .get_version_num(),
            7
        );
    }
    let (headers, body) = request("session", "new-child", Some("grandchild"));
    assert_eq!(
        store
            .inspect("caller", &headers, &body)
            .unwrap()
            .preferred_account_id
            .as_deref(),
        None
    );
}

#[test]
fn ephemeral_fork_keeps_cache_alias_separate_from_logical_session() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (root_headers, root, _) = mapped(
        &store,
        "caller",
        "a",
        "owner-a",
        "root-session",
        "root-thread",
        None,
    );
    let (mut headers, mut body) = request("fork-session", "fork-thread", None);
    headers.insert("session-id", HeaderValue::from_static("root-session"));
    body["prompt_cache_key"] = json!("root-session");
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.session_id.as_deref(), Some("fork-session"));
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["session-id"], root_headers["session-id"]);
    assert_eq!(body["prompt_cache_key"], root["prompt_cache_key"]);
    assert_ne!(
        body["client_metadata"]["session_id"],
        body["prompt_cache_key"]
    );
    assert_ne!(headers["thread-id"], root_headers["thread-id"]);
}

#[test]
fn mappings_survive_restart_account_roundtrip_and_concurrent_connections() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (first, _, _) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    let (other_owner, _, _) = mapped(&store, "caller", "b", "owner-b", "s", "t", None);
    let (other_caller, _, _) = mapped(&store, "other", "a", "owner-a", "s", "t", None);
    assert_ne!(first["thread-id"], other_owner["thread-id"]);
    assert_ne!(first["thread-id"], other_caller["thread-id"]);
    drop(store);
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let path = database.path.clone();
            std::thread::spawn(move || {
                let store = IdentityStore::open(path).unwrap();
                mapped(&store, "caller", "a-renamed", "owner-a", "s", "t", None).0
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap()["thread-id"], first["thread-id"]);
    }
}

#[test]
fn opaque_state_is_preserved_and_cannot_cross_caller_owner_or_turn() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("opaque-token"),
    );
    store.observe_headers(&binding, &response_headers).unwrap();
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"resp-1","status":"completed","output":[{"type":"reasoning","encrypted_content":"ciphertext"}]}})).unwrap();
    let (mut headers, mut body) = request("s", "t", None);
    headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("opaque-token"),
    );
    body["previous_response_id"] = json!("resp-1");
    body["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"reasoning","encrypted_content":"ciphertext"}));
    let original_input = body["input"].clone();
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.preferred_account_id.as_deref(), Some("a"));
    let error = store
        .rewrite(&context, "b", "owner-b", &mut headers, &mut body)
        .unwrap_err();
    assert_eq!(error.code, "upstream_state_owner_changed");
    assert_eq!(headers["thread-id"], "t");
    assert_eq!(
        store.inspect("other", &headers, &body).unwrap_err().code,
        "upstream_state_caller_mismatch"
    );
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["x-codex-turn-state"], "opaque-token");
    assert_eq!(body["previous_response_id"], "resp-1");
    assert_eq!(body["input"], original_input);
    let (mut headers, mut body) = request("s", "t", None);
    headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("opaque-token"),
    );
    let mut canonical: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    canonical["turn_id"] = json!("new-turn");
    body["client_metadata"]["x-codex-turn-metadata"] = json!(canonical.to_string());
    assert_eq!(
        store.inspect("caller", &headers, &body).unwrap_err().code,
        "unknown_upstream_state"
    );
}

#[test]
fn unsupported_opaque_history_fails_before_rewrite_and_delta_does_not_record_it() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    store
        .observe_event(
            &binding,
            &json!({"type":"response.output_text.delta","delta":"resp-secret","id":"resp-secret"}),
        )
        .unwrap();
    let (headers, mut body) = request("s", "t", None);
    body["previous_response_id"] = json!("resp-secret");
    assert_eq!(
        store.inspect("caller", &headers, &body).unwrap_err().code,
        "previous_response_not_found"
    );
    let connection = store.connection.lock().unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM native_opaque_v1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn only_missing_previous_response_requests_client_full_input_replay() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (headers, body) = request("session", "thread", None);
    for headers in [&headers, &HeaderMap::new()] {
        let mut previous = body.clone();
        previous["previous_response_id"] = json!("unknown-checkpoint");
        let error = store.inspect("caller", headers, &previous).unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(error.code, "previous_response_not_found");
    }
    let mut encrypted = body.clone();
    encrypted["input"] = json!([{"type":"reasoning","encrypted_content":"unknown-ciphertext"}]);
    assert_eq!(
        store
            .inspect("caller", &headers, &encrypted)
            .unwrap_err()
            .code,
        "unknown_upstream_state"
    );
    let mut parent = body.clone();
    parent["client_metadata"]["parent_response_id"] = json!("unknown-parent");
    assert_eq!(
        store.inspect("caller", &headers, &parent).unwrap_err().code,
        "unknown_upstream_state"
    );
    let mut turn_headers = headers.clone();
    turn_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("unknown-turn-token"),
    );
    assert_eq!(
        store
            .inspect("caller", &turn_headers, &body)
            .unwrap_err()
            .code,
        "unknown_upstream_state"
    );
}

#[test]
fn capacity_failure_rolls_back_without_rotating_existing_identities() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (first, _, _) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    {
        let connection = store.connection.lock().unwrap();
        connection.execute_batch("CREATE TRIGGER prevent_identity_insert BEFORE INSERT ON native_identity_v1 BEGIN SELECT RAISE(ABORT, 'test capacity'); END;").unwrap();
    }
    let (mut headers, mut body) = request("new-session", "new-thread", None);
    let original_headers = headers.clone();
    let original_body = body.clone();
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert!(store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .is_err());
    assert_eq!(headers, original_headers);
    assert_eq!(body, original_body);
    assert_eq!(
        mapped(&store, "caller", "a", "owner-a", "s", "t", None).0["thread-id"],
        first["thread-id"]
    );
}

#[test]
fn persistent_continuations_outlive_routing_tokens_and_only_completed_responses_resume() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    for kind in ["response.created", "response.failed", "response.incomplete"] {
        store
            .observe_event(&binding, &json!({"type":kind,"response":{"id":kind}}))
            .unwrap();
        let (headers, mut body) = request("s", "t", None);
        body["previous_response_id"] = json!(kind);
        assert_eq!(
            store.inspect("caller", &headers, &body).unwrap_err().code,
            "previous_response_not_found"
        );
    }
    store.observe_event(&binding, &json!({"id":"completed-id","status":"completed","output":[
        {"type":"context_compaction","encrypted_content":"long-lived-cipher"},
        {"type":"message","encrypted_content":"message-cipher","role":"assistant","content":[]}
    ]})).unwrap();
    let mut token_headers = HeaderMap::new();
    token_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("short-lived"),
    );
    store.observe_headers(&binding, &token_headers).unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE native_opaque_v1 SET expires=1", [])
        .unwrap();
    drop(store);
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (mut headers, mut body) = request("s", "t", None);
    body["previous_response_id"] = json!("completed-id");
    body["input"] = json!([
        {"type":"context_compaction","encrypted_content":"long-lived-cipher"},
        {"type":"message","encrypted_content":"message-cipher","role":"assistant","content":[]}
    ]);
    assert_eq!(
        store
            .inspect("caller", &headers, &body)
            .unwrap()
            .state_account_id
            .as_deref(),
        Some("a")
    );
    headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("short-lived"),
    );
    assert_eq!(
        store.inspect("caller", &headers, &body).unwrap_err().code,
        "unknown_upstream_state"
    );
}

#[test]
fn reissued_routing_token_keeps_independent_turn_and_thread_scopes() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("same-upstream-token"),
    );
    for thread in ["one", "two"] {
        let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "s", thread, None);
        store.observe_headers(&binding, &response_headers).unwrap();
    }
    for thread in ["one", "two"] {
        let (mut headers, body) = request("s", thread, None);
        headers.insert(
            "x-codex-turn-state",
            HeaderValue::from_static("same-upstream-token"),
        );
        assert_eq!(
            store
                .inspect("caller", &headers, &body)
                .unwrap()
                .state_account_id
                .as_deref(),
            Some("a")
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("same-upstream-token"),
    );
    assert_eq!(
        store
            .inspect("caller", &headers, &json!({}))
            .unwrap_err()
            .code,
        "unknown_upstream_state"
    );
}

#[test]
fn installation_identity_and_user_content_remain_unchanged() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (mut headers, mut body) = request("s", "t", None);
    headers.insert(
        "x-codex-installation-id",
        HeaderValue::from_static("install-original"),
    );
    body["client_metadata"]["x-codex-installation-id"] = json!("install-original");
    let mut canonical: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    canonical["installation_id"] = json!("install-original");
    body["client_metadata"]["x-codex-turn-metadata"] = json!(canonical.to_string());
    headers.insert(
        "x-codex-turn-metadata",
        HeaderValue::from_str(&canonical.to_string()).unwrap(),
    );
    let input = body["input"].clone();
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.state_account_id, None);
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["x-codex-installation-id"], "install-original");
    assert_eq!(
        body["client_metadata"]["x-codex-installation-id"],
        "install-original"
    );
    let canonical: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(canonical["installation_id"], "install-original");
    assert_eq!(body["input"], input);
}

#[test]
fn generic_stateless_requests_receive_independent_complete_identities() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let original = json!({"model":"test","input":"unchanged user input"});
    let first = store
        .inspect("caller", &HeaderMap::new(), &original)
        .unwrap();
    let second = store
        .inspect("caller", &HeaderMap::new(), &original)
        .unwrap();
    assert_eq!(first.session_id, first.thread_id);
    assert_ne!(first.session_id, second.session_id);
    assert_eq!(
        uuid::Uuid::parse_str(first.session_id.as_deref().unwrap())
            .unwrap()
            .get_version_num(),
        7
    );
    let mut headers = HeaderMap::new();
    let mut body = original.clone();
    store
        .rewrite(&first, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["session-id"], headers["thread-id"]);
    assert_eq!(headers["thread-id"], headers["x-client-request-id"]);
    assert_eq!(
        body["client_metadata"]["session_id"].as_str().unwrap(),
        headers["session-id"].to_str().unwrap()
    );
    assert_eq!(
        body["client_metadata"]["thread_id"].as_str().unwrap(),
        headers["thread-id"].to_str().unwrap()
    );
    assert_eq!(body["input"], original["input"]);
    assert_eq!(first.state_account_id, None);
}

#[test]
fn missing_identity_uses_own_thread_and_never_guesses_parent_session() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("existing-session"));
    let context = store.inspect("caller", &headers, &json!({})).unwrap();
    assert_eq!(context.session_id.as_deref(), Some("existing-session"));
    assert_eq!(context.thread_id, context.session_id);
    headers.clear();
    headers.insert("thread-id", HeaderValue::from_static("child"));
    headers.insert(
        "x-codex-parent-thread-id",
        HeaderValue::from_static("parent"),
    );
    let context = store.inspect("caller", &headers, &json!({})).unwrap();
    assert_eq!(context.thread_id.as_deref(), Some("child"));
    assert_eq!(context.session_id.as_deref(), Some("child"));
    assert_ne!(context.session_id, context.parent_thread_id);
}

#[test]
fn generic_previous_response_recovers_identity_and_opaque_owner_after_restart() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut headers = HeaderMap::new();
    let mut body = json!({"model":"test","input":"first"});
    let original = store.inspect("caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&original, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    let first_headers = headers.clone();
    store
        .observe_event(
            &binding,
            &json!({"object":"response","id":"generic-response","status":"completed","output":[]}),
        )
        .unwrap();
    store.observe_event(&binding, &json!({"object":"response.compaction","id":"compact-output","output":[{"type":"context_compaction","encrypted_content":"generic-compaction"}]})).unwrap();
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("generic-route"),
    );
    store.observe_headers(&binding, &response_headers).unwrap();
    drop(store);
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let mut headers = response_headers;
    let mut body = json!({"model":"test","previous_response_id":"generic-response","input":[{"type":"context_compaction","encrypted_content":"generic-compaction"}]});
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.session_id, original.session_id);
    assert_eq!(context.thread_id, original.thread_id);
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["session-id"], first_headers["session-id"]);
    assert_eq!(headers["thread-id"], first_headers["thread-id"]);
    assert_eq!(headers["x-codex-turn-state"], "generic-route");
    assert_eq!(body["previous_response_id"], "generic-response");
    assert_eq!(body["input"][0]["encrypted_content"], "generic-compaction");
    assert_eq!(
        store
            .inspect(
                "caller",
                &HeaderMap::new(),
                &json!({"previous_response_id":"unknown"})
            )
            .unwrap_err()
            .code,
        "previous_response_not_found"
    );
}

#[test]
fn canonical_metadata_repairs_stale_projections_without_replacing_cache_alias() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (mut headers, mut body) = request("logical-session", "child", Some("parent"));
    let mut canonical: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    canonical["tool_namespaces_info"] = json!({"工具": {"description":"说明 🐈"}});
    body["client_metadata"]["x-codex-turn-metadata"] = json!(canonical.to_string());
    body["client_metadata"]["thread_id"] = json!("old-thread");
    body["client_metadata"]["session_id"] = json!("old-session");
    body["client_metadata"]["turn_id"] = json!("old-turn");
    body["client_metadata"]["x-codex-parent-thread-id"] = json!("old-parent");
    headers.insert("thread-id", HeaderValue::from_static("old-thread"));
    headers.insert(
        "x-client-request-id",
        HeaderValue::from_static("old-request"),
    );
    headers.insert("session-id", HeaderValue::from_static("cache-alias"));
    body["prompt_cache_key"] = json!("cache-alias");
    let context = store.inspect("caller", &headers, &body).unwrap();
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    let mapped: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let projection: Value =
        serde_json::from_str(headers["x-codex-turn-metadata"].to_str().unwrap()).unwrap();
    assert_eq!(headers["thread-id"], headers["x-client-request-id"]);
    assert_eq!(headers["thread-id"].to_str().unwrap(), mapped["thread_id"]);
    assert_eq!(body["client_metadata"]["thread_id"], mapped["thread_id"]);
    assert_eq!(body["client_metadata"]["session_id"], mapped["session_id"]);
    assert_eq!(body["client_metadata"]["turn_id"], mapped["turn_id"]);
    assert_eq!(
        headers["x-codex-parent-thread-id"].to_str().unwrap(),
        mapped["parent_thread_id"]
    );
    assert_eq!(
        body["client_metadata"]["x-codex-parent-thread-id"],
        mapped["parent_thread_id"]
    );
    assert_eq!(
        headers["session-id"].to_str().unwrap(),
        body["prompt_cache_key"]
    );
    assert_ne!(body["prompt_cache_key"], mapped["session_id"]);
    assert!(projection.get("tool_namespaces_info").is_none());
    assert_eq!(
        mapped["tool_namespaces_info"],
        canonical["tool_namespaces_info"]
    );
}

#[test]
fn memory_metadata_keeps_intentional_identity_omissions_and_unicode_header_safe() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (mut headers, mut body) = request("session", "memory-thread", Some("parent"));
    let canonical =
        json!({"request_kind":"memory","turn_id":"turn-memory-thread","label":"记忆 🐈"});
    body["client_metadata"]["x-codex-turn-metadata"] = json!(canonical.to_string());
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert!(context.parent_thread_id.is_none());
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    let mapped: Value = serde_json::from_str(
        body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    for field in ["session_id", "thread_id", "window_id", "context_window_id"] {
        assert!(mapped.get(field).is_none());
    }
    assert!(body["client_metadata"]
        .get("x-codex-parent-thread-id")
        .is_none());
    assert!(!headers.contains_key("x-codex-parent-thread-id"));
    assert_eq!(
        headers["thread-id"].to_str().unwrap(),
        body["client_metadata"]["thread_id"]
    );
    assert_eq!(
        headers["x-codex-window-id"].to_str().unwrap(),
        format!("{}:7", headers["thread-id"].to_str().unwrap())
    );
    let raw = headers["x-codex-turn-metadata"].to_str().unwrap();
    assert!(raw.is_ascii());
    assert_eq!(
        serde_json::from_str::<Value>(raw).unwrap()["label"],
        canonical["label"]
    );
}

#[test]
fn guardian_parent_reference_pins_live_owner_without_allowing_early_resume() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, parent) = mapped(&store, "caller", "a", "owner-a", "session", "parent", None);
    store.observe_event(&parent, &json!({"type":"response.created","response":{"id":"live-response","status":"in_progress"}})).unwrap();
    let (mut headers, mut body) = request("session", "guardian", Some("parent"));
    body["client_metadata"]["parent_response_id"] = json!("live-response");
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert_eq!(
        store
            .rewrite(&context, "b", "owner-b", &mut headers, &mut body)
            .unwrap_err()
            .code,
        "upstream_state_owner_changed"
    );
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(
        body["client_metadata"]["parent_response_id"],
        "live-response"
    );
    let (headers, mut body) = request("session", "parent", None);
    body["previous_response_id"] = json!("live-response");
    assert_eq!(
        store.inspect("caller", &headers, &body).unwrap_err().code,
        "previous_response_not_found"
    );
    drop(store);
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (headers, mut body) = request("session", "guardian", Some("parent"));
    body["client_metadata"]["parent_response_id"] = json!("live-response");
    assert_eq!(
        store
            .inspect("caller", &headers, &body)
            .unwrap()
            .state_account_id
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        store
            .inspect("other-caller", &headers, &body)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
}

#[test]
fn encrypted_function_arguments_keep_their_observed_upstream_owner() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "s", "t", None);
    let item = json!({"type":"function_call","id":"call-item","call_id":"call-1","name":"tool", "arguments":"{}","encrypted_function_args":["cipher-one","cipher-two"]});
    store
        .observe_event(
            &binding,
            &json!({"type":"response.output_item.done","item":item}),
        )
        .unwrap();
    let (mut headers, mut body) = request("s", "child", Some("t"));
    body["input"] = json!([item]);
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert_eq!(
        store
            .rewrite(&context, "b", "owner-b", &mut headers, &mut body)
            .unwrap_err()
            .code,
        "upstream_state_owner_changed"
    );
    store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(body["input"], json!([item]));
}

#[test]
fn switching_owner_preserves_root_and_descendant_graph_across_request_order() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (root_a, _, _) = mapped(&store, "caller", "a", "owner-a", "root", "root", None);
    let (grand_b, _, _) = mapped(
        &store,
        "caller",
        "b",
        "owner-b",
        "root",
        "grand",
        Some("child"),
    );
    let (child_b, _, _) = mapped(
        &store,
        "caller",
        "b",
        "owner-b",
        "root",
        "child",
        Some("root"),
    );
    let (root_b, _, _) = mapped(&store, "caller", "b", "owner-b", "root", "root", None);
    assert_eq!(root_b["session-id"], root_b["thread-id"]);
    assert_ne!(root_b["session-id"], root_a["session-id"]);
    assert_eq!(grand_b["session-id"], root_b["thread-id"]);
    assert_eq!(grand_b["x-codex-parent-thread-id"], child_b["thread-id"]);
    assert_eq!(child_b["x-codex-parent-thread-id"], root_b["thread-id"]);
    let (roundtrip, _, _) = mapped(&store, "caller", "a", "owner-a", "root", "root", None);
    assert_eq!(roundtrip["thread-id"], root_a["thread-id"]);
}

#[test]
fn canonical_identity_ignores_invalid_superseded_flat_and_thread_header() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (mut headers, mut body) = request("session", "thread", None);
    body["client_metadata"]["session_id"] = json!(123);
    body["client_metadata"]["thread_id"] = json!({"stale":true});
    headers.insert(
        "thread-id",
        HeaderValue::from_str(&"x".repeat(MAX_ID_BYTES + 1)).unwrap(),
    );
    let context = store.inspect("caller", &headers, &body).unwrap();
    assert_eq!(context.session_id.as_deref(), Some("session"));
    assert_eq!(context.thread_id.as_deref(), Some("thread"));
    store
        .rewrite(&context, "account", "owner", &mut headers, &mut body)
        .unwrap();
    assert_eq!(
        body["client_metadata"]["thread_id"],
        headers["thread-id"].to_str().unwrap()
    );
    let mut invalid_selected = body.clone();
    invalid_selected["client_metadata"]["x-codex-turn-metadata"] =
        json!(json!({"thread_id":123}).to_string());
    assert_eq!(
        store
            .inspect("caller", &HeaderMap::new(), &invalid_selected)
            .unwrap_err()
            .code,
        "invalid_protocol_identity"
    );
}

#[test]
fn durable_affinity_never_becomes_an_unbounded_soft_route_preference() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    mapped(&store, "caller", "a", "owner-a", "root", "parent", None);
    drop(store);
    let store = IdentityStore::open(database.path.clone()).unwrap();
    for thread in ["parent", "child"] {
        let (headers, body) = request("root", thread, Some("parent"));
        let context = store.inspect("caller", &headers, &body).unwrap();
        assert_eq!(context.preferred_account_id, None);
        assert_eq!(context.state_account_id, None);
    }
    let (_, _, binding) = mapped(&store, "caller", "a", "owner-a", "root", "parent", None);
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"hard-checkpoint","status":"completed","output":[]}})).unwrap();
    let context = store
        .inspect(
            "caller",
            &HeaderMap::new(),
            &json!({"previous_response_id":"hard-checkpoint","input":"next"}),
        )
        .unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
}

#[test]
fn expired_turn_state_keeps_account_owner_evidence_and_only_expires_routing() {
    let database = Database::new();
    let store = IdentityStore::open(database.path.clone()).unwrap();
    let (_, _, original) = mapped(&store, "caller", "account-a", "owner-a", "s", "t", None);
    let mut token = HeaderMap::new();
    token.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("expired-token"),
    );
    store.observe_headers(&original, &token).unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE native_opaque_v1 SET expires=1 WHERE kind='turn_state'",
            [],
        )
        .unwrap();
    // Recording other observations must not erase an expired token's owner.
    store.observe_event(&original, &json!({"type":"response.created","response":{"id":"other-response","status":"in_progress"}})).unwrap();
    let (mut headers, body) = request("s", "t", None);
    headers.extend(token.clone());
    assert_eq!(
        store.inspect("caller", &headers, &body).unwrap_err().code,
        "unknown_upstream_state"
    );
    for owner in ["owner-a", "owner-b"] {
        let (_, _, replacement) = mapped(&store, "caller", "account-b", owner, "s", "t", None);
        assert_eq!(
            store
                .observe_headers(&replacement, &token)
                .unwrap_err()
                .code,
            "upstream_state_owner_conflict"
        );
    }
    store.observe_headers(&original, &token).unwrap();
    assert_eq!(
        store
            .inspect("caller", &headers, &body)
            .unwrap()
            .state_account_id
            .as_deref(),
        Some("account-a")
    );
}
