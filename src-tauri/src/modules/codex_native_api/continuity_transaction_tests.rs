use super::*;
use serde_json::{json, Value};

fn shared_fixture() -> (tempfile::TempDir, IdentityStore, PathBuf, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("identity.sqlite");
    let snapshot = handoff_snapshot_path(&database);
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/modules/codex_native_api/fixtures/legacy_go_reference");
    std::fs::copy(base.join("continuity-v2.snapshot.json"), &snapshot).unwrap();
    let journal = std::fs::read_to_string(base.join("continuity-v2.jsonl")).unwrap();
    let store = IdentityStore::open(database).unwrap();
    (
        directory,
        store,
        snapshot,
        "fixture-v2-scope".into(),
        journal,
    )
}
fn rows(store: &IdentityStore, table: &str) -> i64 {
    store
        .connection
        .lock()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn chunk(
    scope: &str,
    generation: &str,
    seq: u64,
    index: u64,
    commit: bool,
    state: Value,
) -> String {
    let payload = state.to_string();
    json!({"schema_version":2,"scope":scope,"generation":generation,"seq":seq,"payload":payload,"sha256":hash(payload.as_bytes()),"transaction":{"id":"01990000-0000-7000-8000-000000000101","index":index,"commit":commit,"recovery":true}}).to_string()+"\n"
}

#[test]
fn shared_go_transaction_fixture_is_invisible_until_commit_and_reimports_idempotently() {
    let (_directory, store, snapshot, scope, journal) = shared_fixture();
    let lines: Vec<_> = journal.split_inclusive('\n').collect();
    for end in 1..=2 {
        std::fs::write(snapshot.with_extension("jsonl"), lines[..end].concat()).unwrap();
        store.import_handoff(&snapshot, &scope).unwrap();
        for table in [
            "native_identity_v1",
            "native_opaque_v1",
            "native_recovery_commits_v1",
        ] {
            assert_eq!(rows(&store, table), 0);
        }
    }
    std::fs::write(snapshot.with_extension("jsonl"), &journal).unwrap();
    store.import_handoff(&snapshot, &scope).unwrap();
    store.import_handoff(&snapshot, &scope).unwrap();
    assert_eq!(rows(&store, "native_identity_v1"), 1);
    assert_eq!(rows(&store, "native_opaque_v1"), 2);
    assert_eq!(rows(&store, "native_recovery_commits_v1"), 1);
    let request = store.inspect("fixture-caller", &axum::http::HeaderMap::new(), &json!({"previous_response_id":"fixture-response","input":[{"type":"reasoning","encrypted_content":"fixture-ciphertext"}]})).unwrap();
    assert_eq!(request.state_account_id.as_deref(), Some("fixture-account"));
}

#[test]
fn incomplete_chunk_export_retires_the_entire_transaction_and_invalid_order_fails_closed() {
    for invalid in [false, true] {
        let (_directory, store, snapshot, scope, journal) = shared_fixture();
        let lines: Vec<_> = journal.split_inclusive('\n').collect();
        let mut second: Value = serde_json::from_str(lines[1]).unwrap();
        if invalid {
            second["transaction"]["index"] = json!(9);
        }
        std::fs::write(
            snapshot.with_extension("jsonl"),
            format!("{}{}\n", lines[0], second),
        )
        .unwrap();
        if invalid {
            assert!(store.import_handoff(&snapshot, &scope).is_err());
            assert_eq!(rows(&store, "native_identity_v1"), 0);
        } else {
            store.export_handoff(&snapshot, &scope).unwrap();
            assert_eq!(
                std::fs::metadata(snapshot.with_extension("jsonl"))
                    .unwrap()
                    .len(),
                0
            );
            let restored = IdentityStore::open(snapshot.with_extension("restored.sqlite")).unwrap();
            restored.import_handoff(&snapshot, &scope).unwrap();
            assert_eq!(rows(&restored, "native_identity_v1"), 0);
        }
    }
}

#[test]
fn large_legacy_completion_transaction_remains_invisible_until_final_commit() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("identity.sqlite");
    let snapshot = handoff_snapshot_path(&database);
    let scope = handoff_scope(&database);
    let generation = uuid::Uuid::now_v7().to_string();
    std::fs::write(
        &snapshot,
        json!({"schema_version":1,"scope":scope,"generation":generation,"state":{}}).to_string(),
    )
    .unwrap();
    let completion = super::super::super::recovery::Completion {
        id: uuid::Uuid::now_v7().to_string(),
        caller: "caller".into(),
        thread: "thread".into(),
        account: "account".into(),
        owner: "owner".into(),
        completed_at: 100,
    };
    let mut file = File::create(snapshot.with_extension("jsonl")).unwrap();
    for index in 0..3 {
        let opaque: Vec<_> = (index*30000..(index+1)*30000).map(|i| json!({"caller":"caller","kind":"encrypted","digest":format!("{i:064x}"),"account":"account","owner":"owner","session":"session","thread":"thread","expires":0})).collect();
        let line = chunk(
            &scope,
            &generation,
            index + 1,
            index,
            false,
            json!({"opaque":opaque}),
        );
        assert!(line.len() <= MAX_JOURNAL_LINE_BYTES);
        file.write_all(line.as_bytes()).unwrap();
    }
    file.flush().unwrap();
    assert!(file.metadata().unwrap().len() > MAX_JOURNAL_LINE_BYTES as u64);
    let store = IdentityStore::open(database.clone()).unwrap();
    store.import_handoff(&snapshot, &scope).unwrap();
    assert_eq!(rows(&store, "native_opaque_v1"), 0);
    file.write_all(
        chunk(
            &scope,
            &generation,
            4,
            3,
            true,
            json!({"recovery_completed":[completion]}),
        )
        .as_bytes(),
    )
    .unwrap();
    file.flush().unwrap();
    store.import_handoff(&snapshot, &scope).unwrap();
    assert_eq!(rows(&store, "native_opaque_v1"), 90000);
    assert_eq!(rows(&store, "native_recovery_commits_v1"), 1);
}

#[test]
fn forward_fields_are_ignored_but_known_ownership_and_checksum_are_not() {
    for corrupt in [false, true] {
        let (_directory, store, snapshot, scope, journal) = shared_fixture();
        let mut value: Value = serde_json::from_slice(&std::fs::read(&snapshot).unwrap()).unwrap();
        value["future_snapshot"] = json!({"revision":2});
        value["state"]["future_state"] = json!([1, 2]);
        std::fs::write(&snapshot, value.to_string()).unwrap();
        let mut lines = Vec::new();
        for line in journal.lines() {
            let mut entry: Value = serde_json::from_str(line).unwrap();
            entry["future_record"] = json!(true);
            let mut state: Value =
                serde_json::from_str(entry["payload"].as_str().unwrap()).unwrap();
            if let Some(opaque) = state.get_mut("opaque").and_then(Value::as_array_mut) {
                for row in opaque {
                    row["future_row"] = json!("ignored");
                    if corrupt {
                        row["owner"] = json!(123);
                    }
                }
            }
            let payload = state.to_string();
            entry["sha256"] = json!(hash(payload.as_bytes()));
            entry["payload"] = json!(payload);
            lines.push(entry.to_string() + "\n");
        }
        std::fs::write(snapshot.with_extension("jsonl"), lines.concat()).unwrap();
        let result = store.import_handoff(&snapshot, &scope);
        if corrupt {
            assert!(result.is_err());
            assert_eq!(rows(&store, "native_opaque_v1"), 0);
        } else {
            result.unwrap();
            assert_eq!(rows(&store, "native_opaque_v1"), 2);
        }
    }
}

#[test]
fn owner_conflict_in_a_later_chunk_rolls_back_all_earlier_chunks() {
    let (_directory, store, snapshot, scope, journal) = shared_fixture();
    let lines: Vec<_> = journal.split_inclusive('\n').collect();
    let mut conflict: Value = serde_json::from_str(lines[1]).unwrap();
    let mut state: Value = serde_json::from_str(conflict["payload"].as_str().unwrap()).unwrap();
    state["opaque"][0]["owner"] = json!("different-owner");
    let payload = state.to_string();
    conflict["payload"] = json!(payload);
    conflict["sha256"] = json!(hash(payload.as_bytes()));
    conflict["seq"] = json!(3);
    conflict["transaction"]["index"] = json!(2);
    let mut commit: Value = serde_json::from_str(lines[2]).unwrap();
    commit["seq"] = json!(4);
    commit["transaction"]["index"] = json!(3);
    std::fs::write(
        snapshot.with_extension("jsonl"),
        format!("{}{}{}\n{}\n", lines[0], lines[1], conflict, commit),
    )
    .unwrap();
    assert!(store.import_handoff(&snapshot, &scope).is_err());
    for table in [
        "native_identity_v1",
        "native_opaque_v1",
        "native_recovery_commits_v1",
    ] {
        assert_eq!(rows(&store, table), 0);
    }
}
