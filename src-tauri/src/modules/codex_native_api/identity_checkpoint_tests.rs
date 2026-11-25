// Included in identity_rebuild_tests so the real publication/routing helpers
// exercise the self-contained bounded hot cache (including inline compression
// for oversized turns).
#[test]
fn large_checkpoint_survives_restart_rotation_with_inline_compression() {
    let (_temp, path, store) = fixture();
    let original = request(vec![message("user", &"large".repeat(4 * 1024 * 1024))]);
    let first = prepare(&store, "caller", "a", &original, true);
    complete(
        &store,
        &first.0,
        "large-a",
        vec![message("assistant", "first answer")],
    );
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
    drop(store);
    let store = IdentityStore::open(path.clone()).unwrap();
    let followup = continuation("large-a", vec![message("user", "next")]);
    let (context, candidate) = plan(&store, &followup, true);
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    let rotated = rebuilt_plan(&store, "b", candidate.unwrap());
    assert_eq!(rotated.2["input"][0], original["input"][0]);
    assert_eq!(rotated.2["input"][1], message("assistant", "first answer"));
    assert_ne!(first.1["thread-id"], rotated.1["thread-id"]);
    complete(
        &store,
        &rotated.0,
        "large-b",
        vec![message("assistant", "second answer")],
    );
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 2);
    let stored_bytes: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT SUM(bytes) FROM native_rebuild_checkpoints_v1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stored_bytes < 100_000,
        "repetitive large items use lossless inline compression"
    );
    let next = continuation("large-b", vec![message("user", "third")]);
    let (context, candidate) = plan(&store, &next, true);
    assert_eq!(context.state_account_id.as_deref(), Some("b"));
    assert_eq!(candidate.unwrap().body["input"][0], original["input"][0]);
    assert!(store.inspect("foreign", &HeaderMap::new(), &next).is_err());
    let mut wrong_thread = next;
    wrong_thread["client_metadata"]["thread_id"] = json!("other");
    // Inspection may retain the original owner for a server-side reference;
    // it must never authorize importing this checkpoint into another thread.
    let mut wrong_context = store
        .inspect("caller", &HeaderMap::new(), &wrong_thread)
        .unwrap();
    assert!(store
        .configure_rebuild(&mut wrong_context, &HeaderMap::new(), &wrong_thread, true)
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .rewrite(
                &wrong_context,
                "a",
                "owner-a",
                &mut HeaderMap::new(),
                &mut wrong_thread.clone()
            )
            .unwrap_err()
            .code,
        "upstream_state_owner_changed"
    );
}

#[test]
fn checkpoint_failure_and_cancellation_publish_no_partial_checkpoint() {
    let (_temp, path, store) = fixture();
    let original = request(vec![message("user", "transaction test")]);
    let prepared = prepare(&store, "caller", "a", &original, true);
    let event = json!({"type":"response.completed","response":{"id":"atomic-result","status":"completed","output":[message("assistant", "ok")]}});
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_checkpoint_item BEFORE INSERT ON native_rebuild_checkpoints_v1 BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;",
        )
        .unwrap();
    assert_eq!(
        store.commit_rebuild(&prepared.0, &event).unwrap_err().code,
        "identity_store_unavailable"
    );
    for table in [
        "native_rebuild_active_v1",
        "native_rebuild_checkpoints_v1",
    ] {
        assert_eq!(rows(&path, table), 0, "failure must roll back {table}");
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_checkpoint_item;")
        .unwrap();
    store.commit_rebuild(&prepared.0, &event).unwrap();
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
    // Duplicate completion cannot replace the first accepted response.
    let mut duplicate = event.clone();
    duplicate["response"]["output"] = json!([message("assistant", "wrong")]);
    store.commit_rebuild(&prepared.0, &duplicate).unwrap();
    let decoded = checkpoint_store::transcript(
        &Connection::open(&path).unwrap(),
        "caller",
        Some("root"),
        "atomic-result",
    )
    .unwrap()
    .unwrap();
    assert_eq!(decoded.last().unwrap(), &message("assistant", "ok"));
    let cancelled = prepare(&store, "caller", "a", &original, true);
    cancelled.0.abort_rebuild();
    let mut event = event;
    event["response"]["id"] = json!("cancelled-result");
    store.commit_rebuild(&cancelled.0, &event).unwrap();
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
}

#[test]
fn corrupt_checkpoint_payload_fails_closed_without_erasing_history() {
    for corrupt in ["invalid-json", "zstd:corrupt"] {
        let (_temp, path, store) = fixture();
        root(&store);
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE native_rebuild_checkpoints_v1 SET transcript=?1 WHERE response='a-root'",
            [corrupt],
        )
        .unwrap();
        let before = active_row(&path);
        let next = continuation("a-root", vec![message("user", "next")]);
        let error = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &next)
            .unwrap_err();
        assert_eq!(error.code, "identity_store_unavailable");
        assert_eq!(active_row(&path), before);
        assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
    }
}

#[test]
fn checkpoint_load_does_not_hold_the_shared_identity_mutex() {
    let (_temp, path, store) = fixture();
    root(&store);
    let store = Arc::new(store);
    let lock = store.connection.lock().unwrap();
    let worker_store = store.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = worker_store.inspect_for_rebuild(
            "caller",
            &HeaderMap::new(),
            &continuation("a-root", vec![message("user", "next")]),
        );
        let full = worker_store.inspect_for_rebuild(
            "caller",
            &HeaderMap::new(),
            &request(vec![message("user", "full input without a previous ID")]),
        );
        sender.send(result.is_ok() && full.is_ok()).unwrap();
    });
    assert!(receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap());
    drop(lock);
    worker.join().unwrap();
    assert!(path.is_file());
}

#[test]
fn large_parameter_manifest_is_inherited_after_restart_with_inline_compression() {
    let (_temp, path, store) = fixture();
    let mut original = request(vec![message("user", "first")]);
    original["instructions"] = json!("instruction ".repeat(800_000));
    let prepared = prepare(&store, "caller", "a", &original, true);
    complete(
        &store,
        &prepared.0,
        "large-parameters",
        vec![message("assistant", "answer")],
    );
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
    drop(store);
    let store = IdentityStore::open(path).unwrap();
    let next = continuation("large-parameters", vec![message("user", "next")]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &next)
        .unwrap();
    let replay = store
        .expand_http_previous(&mut context, &HeaderMap::new(), &next, &next)
        .unwrap();
    assert_eq!(replay.body["instructions"], original["instructions"]);
    assert_eq!(replay.body["input"].as_array().unwrap().len(), 3);
    assert!(replay.body.get("previous_response_id").is_none());
}

#[test]
fn large_parameter_inheritance_and_planning_do_not_wait_for_identity_mutex() {
    let (_temp, path, store) = fixture();
    let mut original = request(vec![]);
    original["instructions"] = json!("x".repeat(MAX_CHECKPOINT_BYTES + 1));
    let prepared = prepare(&store, "caller", "a", &original, true);
    complete(&store, &prepared.0, "parameter-reader", vec![]);
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 1);
    let next = continuation("parameter-reader", vec![message("user", "next")]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &next)
        .unwrap();
    let store = Arc::new(store);
    let lock = store.connection.lock().unwrap();
    let worker_store = store.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let preview = worker_store
            .preview_rebuild_with_checkpoint_parameters(&context, &HeaderMap::new(), &next)
            .unwrap()
            .unwrap();
        let expanded = worker_store
            .expand_http_previous(&mut context, &HeaderMap::new(), &next, &next)
            .unwrap();
        sender.send((preview.body, expanded.body)).unwrap();
    });
    let result = receiver.recv_timeout(std::time::Duration::from_secs(3));
    drop(lock);
    worker.join().unwrap();
    let (preview, expanded) = result.unwrap();
    assert_eq!(preview["instructions"], original["instructions"]);
    assert_eq!(expanded["instructions"], original["instructions"]);
}
