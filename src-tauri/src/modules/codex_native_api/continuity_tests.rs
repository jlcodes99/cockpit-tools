use super::*;
use axum::http::{HeaderMap, HeaderValue};
use serde_json::{json, Value};

struct Fixture {
    _dir: tempfile::TempDir,
    database: PathBuf,
    snapshot: PathBuf,
    scope: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("identity.sqlite");
        Self {
            snapshot: handoff_snapshot_path(&database),
            scope: handoff_scope(&database),
            database,
            _dir: dir,
        }
    }

    fn store(&self) -> IdentityStore {
        IdentityStore::open(self.database.clone()).unwrap()
    }

    fn snapshot(&self, state: Value) -> String {
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(
            &self.snapshot,
            serde_json::to_vec(&json!({
                "schema_version":1,"scope":self.scope,"generation":generation,"state":state
            }))
            .unwrap(),
        )
        .unwrap();
        generation
    }

    fn journal(&self, generation: &str, seq: u64, state: Value) -> String {
        let payload = serde_json::to_string(&state).unwrap();
        serde_json::to_string(&json!({"schema_version":1,"scope":self.scope,"generation":generation,"seq":seq,"sha256":hash(payload.as_bytes()),"payload":payload})).unwrap() + "\n"
    }
}

fn legacy_state() -> Value {
    json!({
        "mappings":[
            {"caller":"key","owner":"owner-a","original":"root","mapped":"root"},
            {"caller":"key","owner":"owner-a","original":"child","mapped":"child"}
        ],
        "affinities":[{"caller":"key","kind":"thread","original":"child","account":"account-a","owner":"owner-a"}],
        "opaque":[{"caller":"key","kind":"response","digest":hash(b"response-a"),"account":"account-a","owner":"owner-a","session":"root","thread":"child","expires":0},
        {"caller":"key","kind":"encrypted","digest":hash(b"cipher-a"),"account":"account-a","owner":"owner-a","session":"root","thread":"child","expires":0}]
    })
}

#[test]
fn handoff_streams_snapshot_and_journal_beyond_former_file_limit() {
    use std::io::Write;
    let fixture = Fixture::new();
    let generation = fixture.snapshot(json!({}));
    let chunk = vec![b' '; 1024 * 1024];
    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(&fixture.snapshot).unwrap());
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.snapshot)
            .unwrap();
        // Valid JSON whitespace exercises a large persisted file without
        // allocating hundreds of MB of fake identity records in the test.
        for _ in 0..257 {
            file.write_all(&chunk).unwrap();
            hasher.update(&chunk);
        }
    }
    let expected = format!("{:x}", hasher.finalize());
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let actual: String = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT snapshot_sha256 FROM native_handoff_v1 WHERE generation=?1",
            [&generation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        actual, expected,
        "streamed snapshot hashing must include trailing whitespace"
    );

    // Remove the large whitespace for the next import; use a fresh generation,
    // since changing a published generation must still be rejected.
    let generation = fixture.snapshot(json!({}));
    {
        let mut file = File::create(fixture.snapshot.with_extension("jsonl")).unwrap();
        for sequence in 1..=17 {
            // Every individual record stays below 16 MiB. Only lifetime file
            // size crosses the old 256 MiB boundary.
            for _ in 0..15 {
                file.write_all(&chunk).unwrap();
            }
            file.write_all(&chunk[..chunk.len() - 1024]).unwrap();
            file.write_all(fixture.journal(&generation, sequence, json!({})).as_bytes())
                .unwrap();
        }
    }
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let actual: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT seq FROM native_handoff_v1 WHERE generation=?1",
            [&generation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actual, 17);
}

#[test]
fn handoff_lifetime_generation_count_does_not_block_new_history() {
    let fixture = Fixture::new();
    let generation = fixture.snapshot(legacy_state());
    let store = fixture.store();
    {
        let connection = store.connection.lock().unwrap();
        create_cursor_table(&connection).unwrap();
        connection.execute_batch(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1024)
             INSERT INTO native_handoff_v1 SELECT 'historical-scope','historical-'||i,'historical-hash',0 FROM n;",
        ).unwrap();
    }
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let actual: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_handoff_v1 WHERE generation=?1",
            [&generation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actual, 1);
}

#[test]
fn native_handoff_imports_go_passthrough_and_preserves_caller_owner_isolation() {
    let fixture = Fixture::new();
    fixture.snapshot(legacy_state());
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("root"));
    headers.insert("thread-id", HeaderValue::from_static("child"));
    headers.insert("x-codex-parent-thread-id", HeaderValue::from_static("root"));
    let mut body = json!({"model":"gpt-test","previous_response_id":"response-a","input":[{"type":"reasoning","encrypted_content":"cipher-a"}]});
    let context = store.inspect("key", &headers, &body).unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("account-a"));
    store
        .rewrite(&context, "account-a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert_eq!(headers["session-id"], "root");
    assert_eq!(headers["thread-id"], "child");
    assert_eq!(headers["x-codex-parent-thread-id"], "root");
    assert!(store.inspect("other-key", &headers, &body).is_err());
    assert!(store
        .rewrite(&context, "account-b", "owner-b", &mut headers, &mut body)
        .is_err());
}

#[test]
fn native_handoff_journal_is_transactional_idempotent_and_restart_safe() {
    let fixture = Fixture::new();
    let generation = fixture.snapshot(json!({}));
    let committed = fixture.journal(&generation, 1, legacy_state());
    // A crash during the next append does not publish its incomplete record.
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        format!("{committed}{{\"schema_version\":"),
    )
    .unwrap();
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    drop(store);
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    assert_eq!(
        export_state(&store.connection.lock().unwrap())
            .unwrap()
            .opaque
            .len(),
        2
    );
    // Appending a conflicting complete batch rolls back every row in that import.
    let invalid = json!({"mappings":[{"caller":"key","owner":"owner-a","original":"new","mapped":"new"},{"caller":"key","owner":"owner-a","original":"root","mapped":"changed"}]});
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        format!("{committed}{}", fixture.journal(&generation, 2, invalid)),
    )
    .unwrap();
    assert!(store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .is_err());
    assert_eq!(
        export_state(&store.connection.lock().unwrap())
            .unwrap()
            .mappings
            .len(),
        2
    );
}

#[test]
fn native_handoff_rejects_corruption_scope_gaps_and_owner_conflicts() {
    let fixture = Fixture::new();
    let generation = fixture.snapshot(json!({}));
    let store = fixture.store();
    assert!(store
        .import_handoff(&fixture.snapshot, "other-scope")
        .is_err());
    let journal = fixture.snapshot.with_extension("jsonl");
    std::fs::write(&journal, fixture.journal(&generation, 2, legacy_state())).unwrap();
    assert!(store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .is_err());
    let mut line: Value =
        serde_json::from_str(fixture.journal(&generation, 1, legacy_state()).trim()).unwrap();
    line["sha256"] = json!("00");
    std::fs::write(&journal, serde_json::to_string(&line).unwrap() + "\n").unwrap();
    assert!(store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .is_err());
    std::fs::write(&journal, fixture.journal(&generation, 1, legacy_state())).unwrap();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let mut other = legacy_state();
    other["opaque"][0]["owner"] = json!("owner-b");
    other["mappings"] = json!([]);
    std::fs::write(
        &journal,
        format!(
            "{}{}",
            fixture.journal(&generation, 1, legacy_state()),
            fixture.journal(&generation, 2, other)
        ),
    )
    .unwrap();
    assert!(store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .is_err());
    assert_eq!(
        export_state(&store.connection.lock().unwrap())
            .unwrap()
            .opaque[0]
            .owner,
        "owner-a"
    );
}

#[test]
fn native_handoff_roundtrip_and_snapshot_rotation_survive_old_journal() {
    let fixture = Fixture::new();
    let generation = fixture.snapshot(json!({}));
    let old_journal = fixture.journal(&generation, 1, legacy_state());
    std::fs::write(fixture.snapshot.with_extension("jsonl"), &old_journal).unwrap();
    let store = fixture.store();
    store
        .export_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let snapshot = read_snapshot(&fixture.snapshot, &fixture.scope)
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(
        snapshot.predecessor_generation.as_deref(),
        Some(generation.as_str())
    );
    assert_eq!(snapshot.state.mappings.len(), 2);
    assert_ne!(snapshot.generation, generation);
    // Simulate a crash immediately after publishing snapshot but before truncation.
    std::fs::write(fixture.snapshot.with_extension("jsonl"), &old_journal).unwrap();
    let destination = IdentityStore::open(fixture._dir.path().join("recovered.sqlite")).unwrap();
    destination
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    assert_eq!(
        export_state(&destination.connection.lock().unwrap())
            .unwrap()
            .opaque
            .len(),
        2
    );
    // A new Go journal may follow the stale prefix, but never precede it.
    let next = fixture.journal(&snapshot.generation, 1, json!({}));
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        format!("{old_journal}{next}"),
    )
    .unwrap();
    destination
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        format!("{next}{old_journal}"),
    )
    .unwrap();
    assert!(destination
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .is_err());
}

#[test]
fn native_handoff_does_not_reapply_old_snapshot_affinity() {
    let fixture = Fixture::new();
    fixture.snapshot(legacy_state());
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE native_affinity_v1 SET account='account-b',owner='owner-b'",
            [],
        )
        .unwrap();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let state = export_state(&store.connection.lock().unwrap()).unwrap();
    assert_eq!(state.affinities[0].account, "account-b");
}

#[test]
fn native_handoff_imports_production_go_serialization_fixture() {
    // Generated offline with encoding/json using the production Go structs in
    // sidecars/cockpit-cliproxy/continuity_store.go. Keep the actual bytes:
    // nil slices, HTML escaping, nested JSON strings and SHA-256 all matter.
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.snapshot,
        include_bytes!("fixtures/go_handoff_v1.json"),
    )
    .unwrap();
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        include_bytes!("fixtures/go_handoff_v1.jsonl"),
    )
    .unwrap();
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, "go-fixture-scope")
        .unwrap();
    let state = export_state(&store.connection.lock().unwrap()).unwrap();
    assert_eq!(state.mappings.len(), 2);
    assert_eq!(state.affinities.len(), 1);
    assert_eq!(state.opaque.len(), 3);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("go-turn-token"),
    );
    let mut body = json!({
        "model":"test", "previous_response_id":"go-response",
        "client_metadata":{"turn_id":"turn-go"},
        "input":[{"type":"reasoning","encrypted_content":"go-opaque"}]
    });
    let context = store.inspect("caller-go", &headers, &body).unwrap();
    assert_eq!(context.session_id.as_deref(), Some("root<&>"));
    assert_eq!(context.thread_id.as_deref(), Some("child<&>"));
    store
        .rewrite(
            &context,
            "account-go",
            "owner<&>\"子",
            &mut headers,
            &mut body,
        )
        .unwrap();
    assert_eq!(headers["thread-id"], "01990000-0000-7000-8000-000000000012");
    assert_eq!(
        headers["session-id"],
        "01990000-0000-7000-8000-000000000011"
    );
    assert_eq!(body["previous_response_id"], "go-response");
    assert_eq!(body["input"][0]["encrypted_content"], "go-opaque");
}

#[test]
fn native_handoff_imports_shared_go_regression_fixture() {
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.snapshot,
        include_bytes!("fixtures/legacy_go_reference/continuity-go-v1.snapshot.json"),
    )
    .unwrap();
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        include_bytes!("fixtures/legacy_go_reference/continuity-go-v1.jsonl"),
    )
    .unwrap();
    let store = fixture.store();
    store
        .import_handoff(&fixture.snapshot, "fixture-scope")
        .unwrap();
    let context = store
        .inspect(
            "fixture-caller",
            &HeaderMap::new(),
            &json!({"input":[{"type":"reasoning","encrypted_content":"fixture-ciphertext"}]}),
        )
        .unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("fixture-account"));
}

#[test]
fn native_handoff_consecutive_interrupted_rotations_keep_one_predecessor() {
    let fixture = Fixture::new();
    let first = fixture.snapshot(json!({}));
    let first_journal = fixture.journal(&first, 1, legacy_state());
    std::fs::write(fixture.snapshot.with_extension("jsonl"), &first_journal).unwrap();
    let store = fixture.store();
    store
        .export_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    let second = read_snapshot(&fixture.snapshot, &fixture.scope)
        .unwrap()
        .unwrap()
        .0;
    // Rotation one published snapshot but did not clear its prior journal;
    // restarted Go can then append records belonging to the new generation.
    let second_journal = fixture.journal(&second.generation,1,json!({"mappings":[{"caller":"key","owner":"owner-a","original":"later","mapped":"later"}]}));
    std::fs::write(
        fixture.snapshot.with_extension("jsonl"),
        format!("{first_journal}{second_journal}"),
    )
    .unwrap();
    store
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    normalize_journal_for_export(&fixture.snapshot, &second).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.snapshot.with_extension("jsonl")).unwrap(),
        second_journal
    );
    store
        .export_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    // Rotation two also loses its journal-truncate operation. Only the
    // immediate predecessor remains, so fresh storage can recover exactly.
    std::fs::write(fixture.snapshot.with_extension("jsonl"), &second_journal).unwrap();
    let recovered = IdentityStore::open(fixture._dir.path().join("recovered.sqlite")).unwrap();
    recovered
        .import_handoff(&fixture.snapshot, &fixture.scope)
        .unwrap();
    assert_eq!(
        export_state(&recovered.connection.lock().unwrap())
            .unwrap()
            .mappings
            .len(),
        3
    );
}
