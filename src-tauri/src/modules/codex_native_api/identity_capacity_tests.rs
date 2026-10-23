use super::*;
use serde_json::json;

#[test]
fn automatic_history_deduplicates_references_without_committing_unaccepted_ownership() {
    for distinct in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let store = IdentityStore::open(path.clone()).unwrap();
        let body = json!({
            "model":"test", "client_metadata":{"session_id":"root","thread_id":"child"},
            "input": (0..=MAX_REQUEST_OPAQUE_REFS).map(|i| json!({"type":"reasoning","encrypted_content":if distinct { format!("cipher-{i}") } else { "same-cipher".into() }})).collect::<Vec<_>>()
        });
        assert!(serde_json::to_vec(&body).unwrap().len() < 8 * 1024 * 1024);
        let inspected = store.inspect_for_rebuild("caller", &HeaderMap::new(), &body);
        if distinct {
            let error = inspected.unwrap_err();
            assert_eq!(error.status, 413);
            assert_eq!(error.code, "continuation_history_too_large");
        } else {
            let context = inspected.unwrap();
            assert_eq!(context.opaque.len(), 1);
            assert_eq!(context.unregistered_blobs.len(), 1);
            assert!(context.state_account_id.is_none());
        }
        let count: i64 = store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM native_opaque_v1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "inspection must never claim input provenance");
    }
}

fn seed_old_limits(store: &IdentityStore) {
    let mut connection = store.connection.lock().unwrap();
    let tx = connection.transaction().unwrap();
    // Seed real on-disk tables at the former lifetime limits, then exercise the
    // public request/observation/automatic-history and engine handoff paths below.
    tx.execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000)
         INSERT INTO native_identity_v1 SELECT 'archive','archive-owner','id-'||i,'mapped-'||i FROM n;
         WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<50000)
         INSERT INTO native_affinity_v1 SELECT 'archive','thread','id-'||i,'archive-account','archive-owner' FROM n;
         WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000)
         INSERT INTO native_opaque_v1 SELECT 'archive','encrypted',printf('%064x',i),'archive-account','archive-owner','session','thread',NULL,0 FROM n;",
    ).unwrap();
    tx.commit().unwrap();
}

#[test]
fn durable_capacity_keeps_new_requests_automatic_history_and_engine_handoff_usable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    seed_old_limits(&store);

    let mut body = json!({"model":"test","input":"first"});
    let mut headers = HeaderMap::new();
    let identity = store.inspect("caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&identity, "account", "owner", &mut headers, &mut body)
        .unwrap();
    let first_headers = headers.clone();
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"response-new","status":"completed","output":[{"type":"reasoning","encrypted_content":"cipher-new"}]}})).unwrap();

    // Unknown complete history is admitted automatically. Its input provenance
    // and active account are published only after successful upstream completion.
    let original = json!({"model":"test","client_metadata":{"session_id":"legacy-root","thread_id":"legacy-child"},"input":[{"type":"message","role":"user","content":"prior task"},{"type":"reasoning","encrypted_content":"legacy-cipher"}]});
    let mut recovery_body = original.clone();
    let mut recovery_headers = HeaderMap::new();
    let mut recovery = store
        .inspect_for_rebuild("recovery-caller", &recovery_headers, &recovery_body)
        .unwrap();
    assert!(store
        .configure_rebuild(&mut recovery, &recovery_headers, &recovery_body, true)
        .unwrap()
        .is_some());
    let recovered = store
        .rewrite(
            &recovery,
            "original-account",
            "original-owner",
            &mut recovery_headers,
            &mut recovery_body,
        )
        .unwrap();
    assert_ne!(recovery_headers["thread-id"], "legacy-child");
    assert_eq!(recovery_body["input"], original["input"]);
    let recovery_rows = || {
        store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_affinity_v1 WHERE caller='recovery-caller'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(recovery_rows(), 0);
    store.observe_event(&recovered, &json!({"type":"response.completed","response":{"id":"recovered-response","status":"completed","output":[]}})).unwrap();
    assert_eq!(recovery_rows(), 2);

    let snapshot = handoff_snapshot_path(&path);
    let scope = handoff_scope(&path);
    store.export_handoff(&snapshot, &scope).unwrap();
    drop(store);
    let store = IdentityStore::open(path.clone()).unwrap();
    store.import_handoff(&snapshot, &scope).unwrap();
    let other = IdentityStore::open(directory.path().join("roundtrip.sqlite")).unwrap();
    other.import_handoff(&snapshot, &scope).unwrap();
    for current in [&store, &other] {
        let mut followup = json!({"model":"test","previous_response_id":"response-new","input":[{"type":"reasoning","encrypted_content":"cipher-new"}]});
        let mut followup_headers = HeaderMap::new();
        let resumed = current
            .inspect("caller", &followup_headers, &followup)
            .unwrap();
        assert_eq!(resumed.state_account_id.as_deref(), Some("account"));
        assert!(current
            .rewrite(
                &resumed,
                "other-account",
                "other-owner",
                &mut followup_headers,
                &mut followup
            )
            .is_err());
        current
            .rewrite(
                &resumed,
                "account",
                "owner",
                &mut followup_headers,
                &mut followup,
            )
            .unwrap();
        assert_eq!(followup_headers["session-id"], first_headers["session-id"]);
        assert_eq!(followup_headers["thread-id"], first_headers["thread-id"]);
        assert!(current
            .inspect("other-caller", &HeaderMap::new(), &followup)
            .is_err());
        let recovered = current
            .inspect("recovery-caller", &HeaderMap::new(), &original)
            .unwrap();
        assert_eq!(
            recovered.state_account_id.as_deref(),
            Some("original-account")
        );
        let connection = current.connection.lock().unwrap();
        for (table, minimum) in [
            ("native_identity_v1", 100_000_i64),
            ("native_affinity_v1", 50_000),
            ("native_opaque_v1", 100_000),
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert!(count > minimum, "{table} was capped or lost records");
        }
        let archived: String = connection.query_row("SELECT mapped FROM native_identity_v1 WHERE caller='archive' AND owner='archive-owner' AND original='id-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(archived, "mapped-1");
    }
}
