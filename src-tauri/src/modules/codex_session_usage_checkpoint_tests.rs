#[test]
fn parser_checkpoint_resumes_append_and_retains_partial_tail() {
    use std::io::Write;
    let dir = make_temp_dir("usage-checkpoint-append");
    let file = rollout_path(&dir, PARENT_ID);
    write_jsonl(
        &file,
        &[
            session_meta(PARENT_ID),
            turn_context("gpt-5.4"),
            token_count_total(100, 20, 10, "2026-07-10T03:00:02Z"),
        ],
    );
    let mut reader = UsageReaderCache::default();
    let first = parse_codex_file_batch(&file, Some(PARENT_ID.into()), None, &mut reader).unwrap();
    let saved = serde_json::to_string(&first.checkpoint).unwrap();
    let restored: ParserCheckpoint = serde_json::from_str(&saved).unwrap();
    let prior_offset = restored.byte_offset;
    let event = token_count_total(140, 30, 15, "2026-07-10T03:00:03Z").to_string() + "\n";
    let cut = event.len() / 2;
    let mut append = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    append.write_all(event[..cut].as_bytes()).unwrap();
    let partial = parse_codex_file_batch(
        &file,
        Some(PARENT_ID.into()),
        Some(restored.clone()),
        &mut reader,
    )
    .unwrap();
    assert_eq!(partial.checkpoint.byte_offset, prior_offset);
    assert!(!partial.checkpoint.complete);
    assert!(partial.token_events.is_empty());
    append.write_all(event[cut..].as_bytes()).unwrap();
    let next =
        parse_codex_file_batch(&file, Some(PARENT_ID.into()), Some(restored), &mut reader).unwrap();
    assert_eq!(nonzero_deltas(&next), vec![(40, 10, 5)]);
    assert_eq!(next.token_events[0].event_index, Some(2));
    assert_eq!(next.line_offset, first.line_offset + 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parser_batches_resume_without_replaying_history() {
    let dir = make_temp_dir("usage-checkpoint-batches");
    let file = rollout_path(&dir, PARENT_ID);
    let mut lines = vec![session_meta(PARENT_ID), turn_context("gpt-5.4")];
    for n in 1..=100 {
        lines.push(token_count_total(n * 10, 0, n, "2026-07-10T03:00:02Z"));
    }
    write_jsonl(&file, &lines);
    let mut checkpoint = None;
    let mut events = 0;
    let mut batches = 0;
    let mut reader = UsageReaderCache::default();
    loop {
        let parsed =
            parse_codex_file_batch(&file, Some(PARENT_ID.into()), checkpoint, &mut reader).unwrap();
        assert!(parsed.token_events.len() <= INSERT_BATCH_SIZE);
        events += parsed.token_events.len();
        batches += 1;
        if parsed.checkpoint.complete {
            break;
        }
        checkpoint = Some(
            serde_json::from_str(&serde_json::to_string(&parsed.checkpoint).unwrap()).unwrap(),
        );
    }
    assert_eq!(events, 100);
    assert!(batches >= 4);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parent_index_is_bounded_and_replay_cursor_is_a_line_not_match_count() {
    let dir = make_temp_dir("usage-parent-index");
    let file = rollout_path(&dir, PARENT_ID);
    let child_file = rollout_path(&dir, CHILD_ID);
    let mut lines = vec![session_meta(PARENT_ID)];
    for n in 1..=100 {
        lines.push(token_count_total(n * 10, 0, n, "2026-07-10T03:00:02Z"));
    }
    write_jsonl(&file, &lines);
    write_jsonl(
        &child_file,
        &[session_meta(CHILD_ID), lines[20].clone(), lines[80].clone()],
    );
    let child = parse_codex_file(&child_file, Some(CHILD_ID.into())).unwrap();
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    let index = build_rollout_index(&[file.clone()]);
    let cutoff = DateTime::parse_from_rfc3339("2026-07-10T04:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut checkpoint = ParserCheckpoint::default();
    let mut reader = UsageReaderCache::default();
    assert!(matches!(
        resolve_parent_replay(
            &conn,
            PARENT_ID,
            cutoff,
            &index,
            &child.token_events[..1],
            &mut checkpoint,
            &mut reader,
        )
        .unwrap(),
        ParentReplay::Pending
    ));
    loop {
        if let ParentReplay::Matched(n) = resolve_parent_replay(
            &conn,
            PARENT_ID,
            cutoff,
            &index,
            &child.token_events[..1],
            &mut checkpoint,
            &mut reader,
        )
        .unwrap()
        {
            assert_eq!(n, 1);
            break;
        }
    }
    assert_eq!(checkpoint.replay_matched, 21);
    assert!(matches!(
        resolve_parent_replay(
            &conn,
            PARENT_ID,
            cutoff,
            &index,
            &child.token_events[1..],
            &mut checkpoint,
            &mut reader,
        )
        .unwrap(),
        ParentReplay::Matched(1)
    ));
    assert_eq!(checkpoint.replay_matched, 81);
    drop(reader);
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn session_rollup_matches_report_before_and_after_backfill_and_append() {
    let dir = make_temp_dir("usage-rollup-report");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    for n in 1..=5 {
        conn.execute("INSERT INTO session_usage_events VALUES(?1,'i','Instance','s','gpt-5.4',?2,100,20,10,'f')",params![format!("r{n}"),1_780_000_000+n]).unwrap();
    }
    let query = CodexSessionUsageQuery {
        from_timestamp: Some(1_780_000_002),
        to_timestamp: Some(1_780_000_004),
        instance_id: Some("i".into()),
    };
    let before = store.query(&query).unwrap();
    while !SESSION_USAGE_ROLLUP.batch(&conn).unwrap() {}
    let after = store.query(&query).unwrap();
    assert_eq!(before.totals.input_tokens, 300);
    assert_eq!(after.totals.request_count, 3);
    assert_eq!(
        serde_json::to_value(&before.totals).unwrap(),
        serde_json::to_value(&after.totals).unwrap()
    );
    conn.execute("DELETE FROM session_usage_events WHERE request_id='r3'", [])
        .unwrap();
    assert_eq!(store.query(&query).unwrap().totals.request_count, 2);
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rewritten_file_keeps_previous_report_until_atomic_publication() {
    let dir = make_temp_dir("usage-rewrite-snapshot");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    for n in 1..=80 {
        conn.execute("INSERT INTO session_usage_events VALUES(?1,'i','Instance','s','gpt-5.4',?2,100,20,10,'rewritten')",params![format!("r{n}"),1_780_000_000+n]).unwrap();
    }
    let query = CodexSessionUsageQuery::default();
    loop {
        let ready = prepare_rewrite_batch(&conn, "rewritten").unwrap();
        assert_eq!(store.query(&query).unwrap().totals.request_count, 80);
        if ready {
            break;
        }
    }
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM session_usage_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute("INSERT INTO session_usage_events VALUES('r1','i','Instance','s','gpt-5.4',1780000001,200,20,10,'rewritten')",[]).unwrap();
    assert_eq!(
        store.query(&query).unwrap().totals.input_tokens,
        8000,
        "interrupted replacement must not publish partial statistics"
    );
    drop(conn);
    let conn = store.open_conn().unwrap();
    assert_eq!(store.query(&query).unwrap().totals.input_tokens, 8000);
    let tx = conn.unchecked_transaction().unwrap();
    publish_rewrite(&tx, "rewritten", true).unwrap();
    tx.rollback().unwrap();
    assert_eq!(store.query(&query).unwrap().totals.input_tokens, 8000);
    publish_rewrite(&conn, "rewritten", true).unwrap();
    assert_eq!(store.query(&query).unwrap().totals.input_tokens, 200);
    while !cleanup_rewrite_batch(&conn).unwrap() {}
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM session_rewrite_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn truncated_rollout_replaces_usage_and_next_append_remains_incremental() {
    use std::io::Write;
    let dir=make_temp_dir("usage-rewrite-integration");
    let home=dir.join("home");
    let file=rollout_path(&home.join("sessions"),PARENT_ID);
    let store=SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let instances=vec![UsageInstance {id:"i".into(),name:"Instance".into(),data_dir:home}];
    write_jsonl(&file,&[session_meta(PARENT_ID),turn_context("gpt-5.4"),
        token_count_total(100,0,10,"2026-07-10T03:00:02Z"),token_count_total(200,0,20,"2026-07-10T03:00:03Z")]);
    assert!(store.sync(false,&instances).unwrap().errors.is_empty());
    assert_eq!(store.query(&Default::default()).unwrap().totals.input_tokens,200);
    write_jsonl(&file,&[session_meta(PARENT_ID),turn_context("gpt-5.4"),token_count_total(50,0,5,"2026-07-10T03:00:02Z")]);
    assert!(store.sync(false,&instances).unwrap().errors.is_empty());
    let report=store.query(&Default::default()).unwrap();
    assert_eq!(report.totals.input_tokens,50);
    assert_eq!(report.totals.request_count,1);
    let mut append=std::fs::OpenOptions::new().append(true).open(&file).unwrap();
    writeln!(append,"{}",token_count_total(75,0,7,"2026-07-10T03:00:03Z")).unwrap();
    assert_eq!(store.sync(false,&instances).unwrap().imported,1);
    let report=store.query(&Default::default()).unwrap();
    assert_eq!(report.totals.input_tokens,75);
    assert_eq!(report.totals.request_count,2);
    assert_eq!(store.sync(false,&instances).unwrap().imported,0);
    fs::remove_dir_all(dir).unwrap();
}

fn write_compressed_jsonl(path: &Path, values: &[JsonValue]) {
    use std::io::Write;

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = zstd::stream::write::Encoder::new(File::create(path).unwrap(), 0).unwrap();
    for value in values {
        writeln!(encoder, "{value}").unwrap();
    }
    encoder.finish().unwrap();
}

#[test]
fn usage_follows_a_rollout_through_compression_and_archive() {
    let dir = make_temp_dir("usage-compressed-migration");
    let home = dir.join("home");
    let plain = rollout_path(&home.join("sessions/2026/07/10"), PARENT_ID);
    let compressed = plain.with_extension("jsonl.zst");
    let archived = home
        .join("archived_sessions")
        .join(compressed.file_name().unwrap());
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let instances = [UsageInstance {
        id: "i".into(),
        name: "Instance".into(),
        data_dir: home.clone(),
    }];
    let mut lines = vec![session_meta(PARENT_ID), turn_context("gpt-5.4")];
    for n in 1..=70 {
        lines.push(token_count_total(
            n * 100,
            n * 20,
            n * 10,
            "2026-07-10T03:00:02Z",
        ));
    }
    write_jsonl(&plain, &lines[..3]);
    write_compressed_jsonl(&compressed, &lines);
    write_compressed_jsonl(&archived, &lines);
    assert_eq!(collect_codex_session_files(&home), vec![plain.clone()]);
    assert_eq!(store.sync(false, &instances).unwrap().imported, 1);

    fs::remove_file(plain).unwrap();
    assert_eq!(collect_codex_session_files(&home), vec![compressed.clone()]);
    assert!(store.sync(false, &instances).unwrap().errors.is_empty());
    let report = store.query(&Default::default()).unwrap();
    assert_eq!(report.files_tracked, 1);
    assert_eq!(report.totals.request_count, 70);
    assert_eq!(report.totals.total_tokens, 7_700);
    assert_eq!(report.totals.cached_input_tokens, 1_400);
    assert_eq!(store.sync(false, &instances).unwrap().imported, 0);

    fs::remove_file(compressed).unwrap();
    assert_eq!(collect_codex_session_files(&home), vec![archived]);
    assert_eq!(store.sync(false, &instances).unwrap().imported, 0);
    let rebuilt = store.sync(true, &instances).unwrap();
    assert!(rebuilt.errors.is_empty());
    assert_eq!(rebuilt.imported, 70);
    assert_eq!(
        store
            .query(&Default::default())
            .unwrap()
            .totals
            .total_tokens,
        7_700
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rebuilding_with_a_compressed_parent_deduplicates_continuation_history() {
    let dir = make_temp_dir("usage-compressed-parent");
    let home = dir.join("home");
    let archive = home.join("archived_sessions");
    let parent = rollout_path(&archive, PARENT_ID).with_extension("jsonl.zst");
    let child = home.join("sessions").join(format!(
        "rollout-2026-07-10T03-00-00-{PARENT_ID}_{CHILD_ID}.jsonl"
    ));
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let instances = [UsageInstance {
        id: "i".into(),
        name: "Instance".into(),
        data_dir: home,
    }];
    let inherited = token_count_total(100, 20, 10, "2026-07-10T03:00:02Z");
    write_jsonl(
        &child,
        &[
            session_meta_at(PARENT_ID, None, "2026-07-10T03:00:03Z"),
            turn_context("gpt-5.4"),
            inherited.clone(),
            token_count_total(130, 25, 15, "2026-07-10T03:00:04Z"),
        ],
    );
    // Before compressed files were discoverable, continuation logs fell back
    // to counting all snapshots because their parent could not be found.
    assert_eq!(store.sync(false, &instances).unwrap().imported, 2);
    write_compressed_jsonl(&parent, &[session_meta(PARENT_ID), inherited]);

    let rebuilt = store.sync(true, &instances).unwrap();
    assert!(rebuilt.errors.is_empty());
    assert_eq!(rebuilt.deferred_files, 0);
    assert_eq!(rebuilt.imported, 2);
    let report = store.query(&Default::default()).unwrap();
    assert_eq!(report.totals.total_tokens, 145);
    assert_eq!(report.totals.cached_input_tokens, 25);
    assert_eq!(store.sync(false, &instances).unwrap().imported, 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn truncated_compressed_replacement_keeps_previous_usage_visible() {
    let dir = make_temp_dir("usage-invalid-compressed-rewrite");
    let home = dir.join("home");
    let plain = rollout_path(&home.join("sessions"), PARENT_ID);
    let compressed = plain.with_extension("jsonl.zst");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let instances = [UsageInstance {
        id: "i".into(),
        name: "Instance".into(),
        data_dir: home,
    }];
    let original = [
        session_meta(PARENT_ID),
        token_count_total(100, 0, 10, "2026-07-10T03:00:02Z"),
    ];
    write_jsonl(&plain, &original);
    assert_eq!(store.sync(false, &instances).unwrap().imported, 1);
    fs::remove_file(plain).unwrap();

    let mut replacement = vec![session_meta(PARENT_ID)];
    for n in 1..=100 {
        replacement.push(token_count_total(n * 200, 0, n, "2026-07-10T03:00:03Z"));
    }
    replacement.push(json!({ "type": "response_item", "payload": "a".repeat(256 * 1024) }));
    write_compressed_jsonl(&compressed, &replacement);
    let mut invalid = fs::read(&compressed).unwrap();
    invalid.pop().unwrap();
    fs::write(&compressed, invalid).unwrap();
    assert_eq!(store.sync(false, &instances).unwrap().errors.len(), 1);
    assert_eq!(
        store
            .query(&Default::default())
            .unwrap()
            .totals
            .total_tokens,
        110
    );

    write_compressed_jsonl(&compressed, &replacement);
    assert!(store.sync(false, &instances).unwrap().errors.is_empty());
    let report = store.query(&Default::default()).unwrap();
    assert_eq!(report.totals.request_count, 100);
    assert_eq!(report.totals.total_tokens, 20_100);
    fs::remove_dir_all(dir).unwrap();
}
