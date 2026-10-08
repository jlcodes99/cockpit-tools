// Sanitized reproduction of instance 34's metadata: 146716 durable bytes, cursor 146766, ordinal 32.
const LEGACY_ROLLOUT_BYTES: usize = 146_716;
const LEGACY_PROJECTION_OFFSET: i64 = 146_766;

fn complete_projected_rollout(f: &Fixture) -> String {
    let mut records = vec![json!({"type":"session_meta", "ordinal":0, "payload":{"id":ROOT}})];
    for ordinal in 1..32 {
        records.push(json!({"type":"event_msg", "ordinal":ordinal, "payload":{"type":"synthetic", "text":""}}));
    }
    let original = records
        .iter()
        .map(|record| format!("{record}\n"))
        .collect::<String>();
    records.last_mut().unwrap()["payload"]["text"] =
        json!("x".repeat(LEGACY_ROLLOUT_BYTES - original.len()));
    let content = records
        .iter()
        .map(|record| format!("{record}\n"))
        .collect::<String>();
    assert_eq!(content.len(), LEGACY_ROLLOUT_BYTES);
    fs::write(&f.child_rollout, &content).unwrap();
    content
}

fn legacy_projection_fixture(offset: i64) -> (Fixture, String) {
    let f = Fixture::new();
    let content = complete_projected_rollout(&f);
    let history = Connection::open(f.source.join("thread_history_1.sqlite")).unwrap();
    history.execute_batch("\
        CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER NOT NULL, next_rollout_ordinal INTEGER NOT NULL);\
        CREATE TABLE thread_turns (thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL, status TEXT NOT NULL, PRIMARY KEY(thread_id, turn_id));\
        CREATE TABLE thread_items (thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL, item_json TEXT NOT NULL, PRIMARY KEY(thread_id, turn_id, item_id));\
        CREATE TABLE thread_realtime_items (thread_id TEXT NOT NULL, item_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL, item_json TEXT NOT NULL, PRIMARY KEY(thread_id, item_id));\
        CREATE TABLE unrelated_metadata (value TEXT); INSERT INTO unrelated_metadata VALUES ('keep');").unwrap();
    for id in [ROOT, "orphan-cache"] {
        history
            .execute(
                "INSERT INTO thread_history_projection_state VALUES (?1, ?2, 32)",
                rusqlite::params![id, offset],
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_turns VALUES (?1, 'turn', 30, 'completed')",
                [id],
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_items VALUES (?1, 'turn', 'item', 31, 'synthetic')",
                [id],
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_realtime_items VALUES (?1, 'realtime', 31, 'synthetic')",
                [id],
            )
            .unwrap();
    }
    drop(history);
    (f, content)
}

fn source_projection_bytes(f: &Fixture) -> Vec<u8> {
    fs::read(f.source.join("thread_history_1.sqlite")).unwrap()
}

#[test]
fn stale_legacy_projection_rebuilds_only_affected_copy_cache_without_touching_source() {
    let (f, original_rollout) = legacy_projection_fixture(LEGACY_PROJECTION_OFFSET);
    let history_before = source_projection_bytes(&f);
    let state_before = fs::read(f.source.join(STATE_DB)).unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    assert_eq!(
        fs::read(&f.child_rollout).unwrap(),
        original_rollout.as_bytes()
    );
    assert_eq!(
        fs::read(f.target_path(&f.child_rollout)).unwrap(),
        original_rollout.as_bytes()
    );
    assert_eq!(source_projection_bytes(&f), history_before);
    assert_eq!(fs::read(f.source.join(STATE_DB)).unwrap(), state_before);
    let target = Connection::open(f.target.join("thread_history_1.sqlite")).unwrap();
    for table in [
        "thread_history_projection_state",
        "thread_turns",
        "thread_items",
        "thread_realtime_items",
    ] {
        let count: i64 = target
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE thread_id=?1"),
                [ROOT],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "stale {table} must rebuild from copied rollout");
        let orphan_count: i64 = target
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE thread_id='orphan-cache'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan_count, 1, "unrelated orphan cache stays intact");
    }
    assert_eq!(
        target
            .query_row("SELECT value FROM unrelated_metadata", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn projection_recovery_keeps_healthy_cache_and_recovers_complete_midline_cursor() {
    for offset in [LEGACY_ROLLOUT_BYTES as i64, 15] {
        let (f, _) = legacy_projection_fixture(offset);
        let before = source_projection_bytes(&f);
        copy_profile(&f.source, &f.target).unwrap();
        let target = Connection::open(f.target.join("thread_history_1.sqlite")).unwrap();
        let count: i64 = target
            .query_row(
                "SELECT count(*) FROM thread_history_projection_state WHERE thread_id=?1",
                [ROOT],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count,
            if offset == LEGACY_ROLLOUT_BYTES as i64 {
                1
            } else {
                0
            }
        );
        assert_eq!(source_projection_bytes(&f), before);
    }
}

#[test]
fn projection_recovery_rejects_negative_cursors_and_missing_durable_records() {
    for invalid in [
        "negative",
        "truncated_record",
        "truncated_tail",
        "malformed",
        "missing_rollout",
    ] {
        let (f, content) = legacy_projection_fixture(if invalid == "negative" {
            -1
        } else {
            LEGACY_PROJECTION_OFFSET
        });
        match invalid {
            "truncated_record" => {
                let last = content[..content.len() - 1].rfind('\n').unwrap();
                fs::write(&f.child_rollout, &content[..last + 1]).unwrap();
            }
            "truncated_tail" => {
                fs::write(&f.child_rollout, &content[..content.len() - 1]).unwrap();
            }
            "malformed" => {
                fs::write(&f.child_rollout, "{invalid}\n").unwrap();
            }
            "missing_rollout" => {
                fs::remove_file(&f.child_rollout).unwrap();
            }
            _ => {}
        }
        let history_before = source_projection_bytes(&f);
        assert!(
            copy_profile(&f.source, &f.target).is_err(),
            "must not publish {invalid}"
        );
        assert!(!f.target.exists(), "{invalid} leaves no published target");
        assert_eq!(source_projection_bytes(&f), history_before);
    }
}

#[test]
fn projection_recovery_rejects_ordinal_gaps_instead_of_clamping_offset() {
    let (f, content) = legacy_projection_fixture(LEGACY_PROJECTION_OFFSET);
    // Removing a complete interior event still leaves the final ordinal 31 present.
    let altered = content
        .lines()
        .filter(|line| serde_json::from_str::<Value>(line).unwrap()["ordinal"] != json!(12))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    fs::write(&f.child_rollout, altered).unwrap();
    assert!(copy_profile(&f.source, &f.target).is_err());
    assert!(!f.target.exists());
}

#[test]
fn projection_recovery_handles_meta_without_ordinal_but_rejects_legacy_nonordinal_events() {
    for legacy_events in [false, true] {
        let (f, _) = legacy_projection_fixture(LEGACY_PROJECTION_OFFSET);
        let mut records = vec![json!({"type":"session_meta", "payload":{"id":ROOT}})];
        for ordinal in 0..32 {
            let mut record =
                json!({"type":"event_msg", "ordinal":ordinal, "payload":{"type":"synthetic"}});
            if legacy_events {
                record.as_object_mut().unwrap().remove("ordinal");
            }
            records.push(record);
        }
        fs::write(
            &f.child_rollout,
            records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let result = copy_profile(&f.source, &f.target);
        if legacy_events {
            assert!(
                result.is_err(),
                "missing event ordinals cannot prove durable completeness"
            );
            assert!(!f.target.exists());
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn projection_recovery_respects_paginated_history_base_ordinal() {
    let (f, _) = legacy_projection_fixture(LEGACY_PROJECTION_OFFSET);
    let mut parent = format!(
        "{}\n",
        json!({"type":"session_meta", "ordinal":0,"payload":{"id":ROOT}})
    );
    for ordinal in 1..36 {
        parent.push_str(&format!(
            "{}\n",
            json!({"type":"event_msg", "ordinal":ordinal, "payload":{"type":"synthetic"}})
        ));
    }
    fs::write(&f.root_rollout, &parent).unwrap();
    let mut child = format!(
        "{}\n",
        json!({"type":"session_meta", "ordinal":36, "payload":{
            "id":ROOT, "history_mode":"paginated", "history_base":{
                "thread_id":ROOT, "end_byte_offset":parent.len(), "end_ordinal_exclusive":36
            }
        }})
    );
    for ordinal in 37..68 {
        child.push_str(&format!(
            "{}\n",
            json!({"type":"event_msg", "ordinal":ordinal, "payload":{"type":"synthetic"}})
        ));
    }
    fs::write(&f.child_rollout, &child).unwrap();
    let history = Connection::open(f.source.join("thread_history_1.sqlite")).unwrap();
    history
        .execute(
            "UPDATE thread_history_projection_state SET next_rollout_ordinal=68 WHERE thread_id=?1",
            [ROOT],
        )
        .unwrap();
    drop(history);
    copy_profile(&f.source, &f.target).unwrap();
    assert_eq!(
        fs::read(f.target_path(&f.child_rollout)).unwrap(),
        child.as_bytes()
    );
    assert_eq!(
        fs::read(f.target_path(&f.root_rollout)).unwrap(),
        parent.as_bytes()
    );
}

#[test]
fn projection_recovery_rejects_unknown_cache_schema_without_publishing() {
    for missing in ["next_ordinal", "cache_columns"] {
        let (f, _) = legacy_projection_fixture(LEGACY_PROJECTION_OFFSET);
        let history = Connection::open(f.source.join("thread_history_1.sqlite")).unwrap();
        if missing == "next_ordinal" {
            history
                .execute_batch(
                    "ALTER TABLE thread_history_projection_state DROP COLUMN next_rollout_ordinal;",
                )
                .unwrap();
        } else {
            history.execute_batch("DROP TABLE thread_realtime_items; CREATE TABLE thread_realtime_items (thread_id TEXT, unknown_layout TEXT);").unwrap();
        }
        drop(history);
        let before = source_projection_bytes(&f);
        assert!(
            copy_profile(&f.source, &f.target).is_err(),
            "unknown {missing} is not recoverable"
        );
        assert!(!f.target.exists());
        assert_eq!(source_projection_bytes(&f), before);
    }
}
