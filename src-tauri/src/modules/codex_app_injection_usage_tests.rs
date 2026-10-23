use super::*;
use std::io::Write;

const THREAD_A: &str = "01976f45-9920-7000-8000-000000000001";
const THREAD_B: &str = "01976f45-9920-7000-8000-000000000002";

fn token(seconds: u32, output: u64, input: u64, cached: u64) -> Value {
    json!({
        "timestamp": format!("2026-10-20T00:00:{seconds:02}Z"),
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": input, "cached_input_tokens": cached,
                    "output_tokens": 20, "total_tokens": input + 20,
                },
                "total_token_usage": {
                    "input_tokens": input, "cached_input_tokens": cached,
                    "output_tokens": output, "total_tokens": input + output,
                },
                "model_context_window": 200000,
            },
        },
    })
}

fn line(value: Value) -> String {
    format!("{value}\n")
}

fn rollout(profile: &Path, thread: &str, content: &str) -> PathBuf {
    let directory = profile.join("sessions/2026/10/20");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("rollout-2026-10-20T00-00-00-{thread}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

fn index_path(profile: &Path, thread: &str, rollout: &Path) -> Connection {
    let db = Connection::open(profile.join("state_5.sqlite")).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (?1, ?2)",
        [thread, rollout.to_str().unwrap()],
    )
    .unwrap();
    db
}

#[test]
fn usage_snapshot_is_numeric_only_and_deduplicates_accounting_snapshots() {
    let profile = tempfile::tempdir().unwrap();
    let mut duplicate = token(11, 100, 1000, 800);
    duplicate["payload"]["private_text"] = json!("must never leave this reader");
    let content =
        line(token(10, 100, 1000, 800)) + &line(duplicate) + &line(token(20, 200, 2000, 1500));
    let path = rollout(profile.path(), THREAD_A, &content);
    let _db = index_path(profile.path(), THREAD_A, &path);
    let snapshot = read_usage_snapshot(profile.path(), THREAD_A).unwrap();
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(snapshot.events[0]["t"], "2026-10-20T00:00:10Z");
    assert_eq!(snapshot.events[1]["cached_input_tokens"], 1500);
    assert_eq!(snapshot.events[1]["context_window"], 200000);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("must never"));
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(serialized["threadId"], THREAD_A);
    assert!(serialized.get("currentTurnId").is_some());
}

#[test]
fn usage_resumed_rollout_reads_indexed_and_unindexed_files() {
    for indexed in [true, false] {
        let profile = tempfile::tempdir().unwrap();
        let content = line(token(10, 100, 1000, 800)) + &line(token(20, 200, 2000, 1800));
        let path = rollout(profile.path(), THREAD_A, &content);
        let resumed = path.with_file_name(format!(
            "rollout-2026-10-20T00-00-00-{THREAD_A}_{THREAD_B}.jsonl"
        ));
        fs::rename(path, &resumed).unwrap();
        let _db = indexed.then(|| index_path(profile.path(), THREAD_A, &resumed));
        let snapshot = read_usage_snapshot(profile.path(), THREAD_A).unwrap();
        assert_eq!(snapshot.thread_id, THREAD_A);
        assert_eq!(snapshot.events.len(), 2, "indexed={indexed}");
        assert_eq!(snapshot.events[1]["turn_output_tokens"], 200);
        assert_eq!(snapshot.events[1]["cached_input_tokens"], 1800);
        let mut file = fs::OpenOptions::new().append(true).open(&resumed).unwrap();
        file.write_all(line(token(30, 300, 3000, 2700)).as_bytes())
            .unwrap();
        assert_eq!(
            read_usage_snapshot(profile.path(), THREAD_A)
                .unwrap()
                .events
                .len(),
            3,
            "resumed cached path must keep receiving samples"
        );
        // The appended UUID is not the conversation identity.
        assert!(read_usage_snapshot(profile.path(), THREAD_B)
            .unwrap()
            .events
            .is_empty());
    }
}

#[test]
fn usage_index_rotation_overrides_existing_cached_file_and_survives_db_busy() {
    let profile = tempfile::tempdir().unwrap();
    let original = rollout(profile.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let db = index_path(profile.path(), THREAD_A, &original);
    assert_eq!(
        read_usage_snapshot(profile.path(), THREAD_A)
            .unwrap()
            .events[0]["turn_output_tokens"],
        100
    );
    for (suffix, output) in [(THREAD_B, 200), (THREAD_A, 300)] {
        let rotated = original.with_file_name(format!(
            "rollout-2026-10-20T00-00-00-{THREAD_A}_{suffix}.jsonl"
        ));
        fs::write(&rotated, line(token(20, output, 2000, 1800))).unwrap();
        db.execute(
            "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
            [rotated.to_str().unwrap(), THREAD_A],
        )
        .unwrap();
        let snapshot = read_usage_snapshot(profile.path(), THREAD_A).unwrap();
        assert_eq!(snapshot.events[0]["turn_output_tokens"], output);
        assert!(original.exists(), "old rollout is retained by the client");
    }
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let snapshot = read_usage_snapshot(profile.path(), THREAD_A).unwrap();
    assert_eq!(snapshot.events[0]["turn_output_tokens"], 300);
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn usage_resumed_filename_rejects_malformed_suffixes_and_wrong_thread() {
    for tail in [
        format!("{THREAD_A}_not-a-uuid"),
        format!("{THREAD_A}_{}", THREAD_B.replace('-', "")),
        format!("{THREAD_A}_"),
        format!("{THREAD_A}_{THREAD_B}_extra"),
        format!("{THREAD_B}_{THREAD_A}"),
    ] {
        assert!(!rollout_filename_matches(
            &format!("rollout-2026-10-20T00-00-00-{tail}.jsonl"),
            THREAD_A
        ));
    }
    let profile = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let path = rollout(other.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let resumed = path.with_file_name(format!(
        "rollout-2026-10-20T00-00-00-{THREAD_A}_{THREAD_B}.jsonl"
    ));
    fs::rename(path, &resumed).unwrap();
    let _db = index_path(profile.path(), THREAD_A, &resumed);
    assert!(read_usage_snapshot(profile.path(), THREAD_A)
        .unwrap()
        .events
        .is_empty());
}

#[cfg(unix)]
#[test]
fn usage_resumed_symlinks_cannot_alias_another_thread() {
    use std::os::unix::fs::symlink;
    let profile = tempfile::tempdir().unwrap();
    let other = rollout(profile.path(), THREAD_B, &line(token(1, 100, 1000, 800)));
    let alias = other.with_file_name(format!(
        "rollout-2026-10-20T00-00-00-{THREAD_A}_{THREAD_B}.jsonl"
    ));
    symlink(&other, &alias).unwrap();
    let _db = index_path(profile.path(), THREAD_A, &alias);
    assert!(read_usage_snapshot(profile.path(), THREAD_A)
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn usage_missing_counters_remain_unknown_and_differ_from_explicit_zero() {
    let mut missing = token(1, 0, 1000, 0);
    missing["payload"]["info"]["last_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cached_input_tokens");
    missing["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("output_tokens");
    let (unknown, unknown_signature) = usage_event(&missing, None).unwrap();
    assert!(unknown["cached_input_tokens"].is_null());
    assert!(unknown["turn_output_tokens"].is_null());
    let (zero, zero_signature) = usage_event(&token(1, 0, 1000, 0), None).unwrap();
    assert_eq!(zero["cached_input_tokens"], 0);
    assert_eq!(zero["turn_output_tokens"], 0);
    assert_ne!(unknown_signature, zero_signature);
    let content = line(missing) + &line(token(1, 0, 1000, 0));
    let (events, _) = parse_usage_window(content.as_bytes(), false, Instant::now()).unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn usage_task_boundaries_survive_completion_and_reset_before_new_tokens() {
    let task = |id: &str, kind: &str| {
        json!({
            "type": "event_msg", "timestamp": "2026-10-20T00:00:00Z",
            "payload": {"type": kind, "turn_id": id},
        })
    };
    let content = line(task("first", "task_started"))
        + &line(token(1, 100, 1000, 800))
        + &line(task("first", "task_complete"))
        + &line(task("second", "task_started"));
    let (events, current) = parse_usage_window(content.as_bytes(), false, Instant::now()).unwrap();
    assert_eq!(current.as_deref(), Some("second"));
    assert_eq!(events[0]["turn_id"], "first");
    assert!(!events.iter().any(|event| event["turn_id"] == "second"));
    let content =
        content + &line(token(20, 120, 2000, 900)) + &line(task("second", "task_complete"));
    let (events, current) = parse_usage_window(content.as_bytes(), false, Instant::now()).unwrap();
    assert_eq!(current.as_deref(), Some("second"));
    assert_eq!(events[1]["turn_id"], "second");
}

#[test]
fn usage_tail_expands_past_large_tool_results_and_ignores_bad_or_partial_lines() {
    let profile = tempfile::tempdir().unwrap();
    let content = line(token(1, 100, 1000, 800))
        + &line(json!({"type": "response_item", "payload": "x".repeat(700 * 1024)}))
        + "{bad \"token_count\"}\n"
        + &line(token(20, 200, 2000, 1600))
        + "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"";
    let path = rollout(profile.path(), THREAD_A, &content);
    let mut file = File::open(path).unwrap();
    let (events, current) =
        read_usage_tail(&mut file, content.len() as u64, Instant::now()).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["turn_output_tokens"], 100);
    assert_eq!(events[1]["turn_output_tokens"], 200);
    assert_eq!(current, None);
}

#[test]
fn usage_tail_never_reads_before_eight_megabytes_and_caps_events() {
    let profile = tempfile::tempdir().unwrap();
    let path = rollout(profile.path(), THREAD_A, &line(token(0, 10, 1000, 800)));
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&vec![b'x'; TAIL_MAX_BYTES as usize + 1])
        .unwrap();
    file.write_all(b"\n").unwrap();
    file.write_all(line(token(20, 20, 1000, 900)).as_bytes())
        .unwrap();
    drop(file);
    let mut file = File::open(&path).unwrap();
    let length = file.metadata().unwrap().len();
    let (events, _) = read_usage_tail(&mut file, length, Instant::now()).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["turn_output_tokens"], 20);
    let content: String = (0..40)
        .map(|n| line(token(n, n as u64 + 1, 1000, 900)))
        .collect();
    let (events, _) = parse_usage_window(content.as_bytes(), false, Instant::now()).unwrap();
    assert_eq!(events.len(), EVENT_LIMIT);
    assert_eq!(events[0]["turn_output_tokens"], 17);
}

#[test]
fn usage_rejects_cross_profile_paths_parent_escape_and_wrong_thread() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let path = rollout(second.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let _db = index_path(first.path(), THREAD_A, &path);
    assert!(read_usage_snapshot(first.path(), THREAD_A)
        .unwrap()
        .events
        .is_empty());
    let local = rollout(first.path(), THREAD_B, &line(token(1, 100, 1000, 800)));
    let roots = session_roots(first.path()).unwrap();
    assert!(validate_rollout_path(first.path(), &roots, &local, THREAD_A).is_none());
    assert!(validate_rollout_path(
        first.path(),
        &roots,
        Path::new("sessions/../state_5.sqlite"),
        THREAD_A
    )
    .is_none());
    for id in [
        "../escape",
        "client-new-thread:123",
        "01976f45992070008000000000000001",
    ] {
        assert!(read_usage_snapshot(first.path(), id).is_err());
    }
}

#[cfg(unix)]
#[test]
fn usage_never_follows_session_or_database_symlinks_outside_profile() {
    use std::os::unix::fs::symlink;
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let path = rollout(second.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let _db = index_path(second.path(), THREAD_A, &path);
    symlink(
        second.path().join("sessions"),
        first.path().join("sessions"),
    )
    .unwrap();
    symlink(
        second.path().join("state_5.sqlite"),
        first.path().join("state_5.sqlite"),
    )
    .unwrap();
    assert!(read_usage_snapshot(first.path(), THREAD_A)
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn usage_missing_new_session_and_draft_are_isolated_from_previous_thread() {
    let profile = tempfile::tempdir().unwrap();
    rollout(profile.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let first = read_usage_snapshot(profile.path(), &format!("local:{THREAD_A}")).unwrap();
    let second = read_usage_snapshot(profile.path(), THREAD_B).unwrap();
    assert_eq!(first.events.len(), 1);
    assert_eq!(second.thread_id, THREAD_B);
    assert!(second.events.is_empty());
    assert!(read_usage_snapshot(profile.path(), "local:client-new-thread:test").is_err());
}

#[test]
fn usage_cache_never_shares_same_thread_id_across_profiles() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    rollout(first.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    rollout(second.path(), THREAD_A, &line(token(1, 900, 9000, 8000)));
    let a = read_usage_snapshot(first.path(), THREAD_A).unwrap();
    let b = read_usage_snapshot(second.path(), THREAD_A).unwrap();
    assert_eq!(a.events[0]["turn_output_tokens"], 100);
    assert_eq!(b.events[0]["turn_output_tokens"], 900);
    assert_eq!(
        read_usage_snapshot(first.path(), THREAD_A).unwrap().events[0]["turn_output_tokens"],
        100
    );
}

#[test]
fn usage_incomplete_search_returns_error_instead_of_negative_cache() {
    let profile = tempfile::tempdir().unwrap();
    let directory = profile.path().join("sessions");
    fs::create_dir(&directory).unwrap();
    for index in 0..=MAX_SCAN_ENTRIES {
        fs::write(directory.join(format!("unrelated-{index}.jsonl")), "").unwrap();
    }
    assert!(read_usage_snapshot(profile.path(), THREAD_A).is_err());
    let key = (profile.path().canonicalize().unwrap(), THREAD_A.to_string());
    assert!(cached_usage(&key).is_none());
}

#[test]
fn usage_busy_database_falls_back_without_waiting_for_writer() {
    let profile = tempfile::tempdir().unwrap();
    let path = rollout(profile.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    let db = index_path(profile.path(), THREAD_A, &path);
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let started = Instant::now();
    let snapshot = read_usage_snapshot(profile.path(), THREAD_A).unwrap();
    assert_eq!(snapshot.events.len(), 1);
    assert!(started.elapsed() < Duration::from_secs(1));
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn usage_cached_path_updates_after_append_and_negative_cache_is_bounded() {
    let profile = tempfile::tempdir().unwrap();
    let path = rollout(profile.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    assert_eq!(
        read_usage_snapshot(profile.path(), THREAD_A)
            .unwrap()
            .events
            .len(),
        1
    );
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(line(token(20, 200, 2000, 1800)).as_bytes())
        .unwrap();
    assert_eq!(
        read_usage_snapshot(profile.path(), THREAD_A)
            .unwrap()
            .events
            .len(),
        2
    );
    assert!(read_usage_snapshot(profile.path(), THREAD_B)
        .unwrap()
        .events
        .is_empty());
    let key = (profile.path().canonicalize().unwrap(), THREAD_B.to_string());
    assert!(cached_usage(&key).unwrap().path.is_none());
    rollout(profile.path(), THREAD_B, &line(token(1, 300, 1000, 800)));
    assert!(read_usage_snapshot(profile.path(), THREAD_B)
        .unwrap()
        .events
        .is_empty());
    {
        let mut cache = CACHE.get().unwrap().lock().unwrap();
        cache.get_mut(&key).unwrap().checked_at = Instant::now() - NEGATIVE_CACHE_TTL;
    }
    assert_eq!(
        read_usage_snapshot(profile.path(), THREAD_B)
            .unwrap()
            .events[0]["turn_output_tokens"],
        300
    );
}

#[test]
fn usage_archived_and_nested_sqlite_index_are_supported() {
    let profile = tempfile::tempdir().unwrap();
    let original = rollout(profile.path(), THREAD_A, &line(token(1, 100, 1000, 800)));
    fs::create_dir_all(profile.path().join("archived_sessions")).unwrap();
    let archived = profile
        .path()
        .join("archived_sessions")
        .join(original.file_name().unwrap());
    fs::rename(original, &archived).unwrap();
    let db = index_path(profile.path(), THREAD_A, &archived);
    drop(db);
    fs::create_dir_all(profile.path().join("sqlite")).unwrap();
    fs::rename(
        profile.path().join("state_5.sqlite"),
        profile.path().join("sqlite/state_5.sqlite"),
    )
    .unwrap();
    assert_eq!(
        read_usage_snapshot(profile.path(), THREAD_A)
            .unwrap()
            .events
            .len(),
        1
    );
}

#[test]
fn usage_window_budget_failure_is_explicit() {
    let started = Instant::now() - READ_BUDGET - Duration::from_millis(1);
    assert!(parse_usage_window(line(token(1, 100, 1000, 800)).as_bytes(), false, started).is_err());
}
