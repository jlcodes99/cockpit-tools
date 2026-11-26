#[test]
fn request_pages_use_stable_ties_and_the_published_rollout_snapshot() {
    let dir = make_temp_dir("usage-request-pages");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    conn.execute("INSERT INTO session_log_sync VALUES ('f','i',0,0,0)", [])
        .unwrap();
    save_usage_project(
        &conn,
        "f",
        "session",
        Some(&RolloutUsageProject {
            key: "/work/project".into(),
            path: "/work/project".into(),
            title: "Example session".into(),
        }),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_log_sync VALUES ('other','i',0,0,0)",
        [],
    )
    .unwrap();
    save_usage_project(&conn, "other", "other", None).unwrap();
    set_meta(&conn, PROJECT_BACKFILL_COMPLETE, "1").unwrap();
    for n in 0..=51 {
        conn.execute(
            "INSERT INTO session_usage_events
             VALUES (?1,'i','Instance',?2,'gpt-5.4',1780000000,100,20,10,?3)",
            params![
                format!("r{n:02}"),
                if n == 0 { "other" } else { "session" },
                if n == 0 { "other" } else { "f" },
            ],
        )
        .unwrap();
    }
    let query = CodexSessionUsageQuery {
        model: Some("gpt-5.4".into()),
        project_key: Some("/work/project".into()),
        session_id: Some("session".into()),
        include_projects: true,
        ..Default::default()
    };
    let names = HashMap::from([("i".into(), "Renamed".into())]);
    let page = query_usage_request_page(&conn, &query, None, &names).unwrap();
    assert_eq!(page.rows.len(), 50);
    assert_eq!(page.rows[0].request_id, "r51");
    assert_eq!(page.rows[0].instance_name, "Renamed");
    assert_eq!(page.rows[0].session_title, "Example session");

    while !prepare_rewrite_batch(&conn, "f").unwrap() {}
    conn.execute(
        "INSERT INTO session_usage_events
         VALUES ('r51','i','Instance','session','gpt-5.4',1780000000,500,20,10,'f')",
        [],
    )
    .unwrap();
    let next = query_usage_request_page(&conn, &query, page.next_cursor.as_ref(), &names).unwrap();
    assert_eq!(next.rows.len(), 1);
    assert_eq!(next.rows[0].request_id, "r01");
    assert!(next.next_cursor.is_none());
    let report = store.query(&query).unwrap();
    assert_eq!(report.totals.request_count, 51);
    assert_eq!(report.by_project[0].key, "/work/project");
    assert!(report.by_project[0].estimated_cost_usd.is_some());

    publish_rewrite(&conn, "f", true).unwrap();
    let published = query_usage_request_page(&conn, &query, None, &names).unwrap();
    assert_eq!(published.rows.len(), 1);
    assert_eq!(published.rows[0].input_tokens, 500);
    assert_eq!(store.query(&query).unwrap().totals.request_count, 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn project_backfill_resumes_without_overwriting_new_metadata_or_hiding_unknown_usage() {
    let dir = make_temp_dir("usage-project-backfill");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    let path = dir.join("rollout.jsonl");
    let missing = dir.join("removed.jsonl");
    let too_large = dir.join("large.jsonl");
    write_jsonl(
        &path,
        &[serde_json::json!({
            "type": "session_meta", "payload": { "cwd": "/work/one", "title": "First" }
        })],
    );
    fs::write(&too_large, vec![b' '; 1024 * 1024 + 1]).unwrap();
    for (i, file) in [&path, &missing, &too_large].into_iter().enumerate() {
        let file = file.to_str().unwrap();
        conn.execute("INSERT INTO session_log_sync VALUES (?1,'i',0,0,0)", [file])
            .unwrap();
        conn.execute(
            "INSERT INTO session_usage_events
             VALUES (?1,'i','Instance','session','gpt-5.4',1780000000,100,20,10,?2)",
            params![format!("request{i}"), file],
        )
        .unwrap();
    }
    let query = CodexSessionUsageQuery {
        include_projects: true,
        ..Default::default()
    };
    let before = store.query(&query).unwrap();
    assert_eq!(before.totals.request_count, 3);
    assert!(before.projects_pending);
    assert_eq!(before.by_project[0].key, "");
    assert!(!backfill_usage_projects(&conn).unwrap());

    save_usage_project(
        &conn,
        path.to_str().unwrap(),
        "session",
        Some(&RolloutUsageProject {
            key: "/work/new".into(),
            path: "/work/new".into(),
            title: "New metadata".into(),
        }),
    )
    .unwrap();
    drop(conn);
    let conn = store.open_conn().unwrap();
    while !backfill_usage_projects(&conn).unwrap() {}
    assert!(backfill_usage_projects(&conn).unwrap());
    let after = store.query(&query).unwrap();
    assert_eq!(after.totals.request_count, before.totals.request_count);
    assert!(!after.projects_pending);
    assert_eq!(
        after
            .by_project
            .iter()
            .find(|row| row.key == "/work/new")
            .unwrap()
            .request_count,
        1
    );
    assert_eq!(
        store
            .query(&CodexSessionUsageQuery {
                project_key: Some(String::new()),
                ..query
            })
            .unwrap()
            .totals
            .request_count,
        2
    );
    conn.execute(
        "DELETE FROM session_log_sync WHERE file_path=?1",
        [path.to_str().unwrap()],
    )
    .unwrap();
    let metadata_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_usage_projects WHERE project_key='/work/new'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(metadata_count, 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn request_page_attaches_first_response_and_tps_from_gateway_summaries_and_builds_hourly_breakdown() {
    let dir = make_temp_dir("usage-request-perf");
    let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
    let conn = store.open_conn().unwrap();
    let base_ts = 1_780_002_000_i64; // exact hour boundary (divisible by 3600)
    conn.execute("INSERT INTO session_log_sync VALUES ('f','i',0,0,0)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO session_usage_events
         VALUES ('r1','i','Instance','session-1','gpt-5.4',?1,1000,400,250,'f')",
        params![base_ts + 120],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_usage_events
         VALUES ('r2','i','Instance','session-1','gpt-5.4',?1,800,200,120,'f')",
        params![base_ts + 3720],
    )
    .unwrap();

    let query = CodexSessionUsageQuery {
        from_timestamp: Some(base_ts),
        to_timestamp: Some(base_ts + 7200),
        ..Default::default()
    };
    let report = store.query(&query).unwrap();
    assert_eq!(report.by_hour.len(), 2);
    assert_eq!(report.by_hour[0].key, base_ts.to_string());
    assert_eq!(report.by_hour[0].total_tokens, 1250);
    assert_eq!(report.by_hour[1].key, (base_ts + 3600).to_string());
    assert_eq!(report.by_hour[1].total_tokens, 920);

    let mut page = query_usage_request_page(&conn, &query, None, &HashMap::new()).unwrap();
    let logs_conn = Connection::open_in_memory().unwrap();
    logs_conn
        .execute_batch(
            "CREATE TABLE request_display_summaries (
                client_thread_id TEXT,
                timestamp INTEGER,
                dispatched INTEGER,
                data_json TEXT
            );",
        )
        .unwrap();
    logs_conn
        .execute(
            "INSERT INTO request_display_summaries VALUES (?1, ?2, 1, ?3)",
            params![
                "session-1",
                (base_ts + 122) * 1000,
                serde_json::json!({
                    "success": true,
                    "outputTokens": 250,
                    "firstResponseMs": 640,
                    "outputGenerationMs": 2500
                })
                .to_string()
            ],
        )
        .unwrap();
    attach_gateway_request_performance_from_conn(&logs_conn, &mut page.rows);
    let matched = page.rows.iter().find(|row| row.request_id == "r1").unwrap();
    assert_eq!(matched.first_response_ms, Some(640));
    assert_eq!(matched.output_generation_ms, Some(2500));
    assert!((matched.tokens_per_second.unwrap() - 100.0).abs() < 1e-6);

    let unmatched = page.rows.iter().find(|row| row.request_id == "r2").unwrap();
    assert_eq!(unmatched.first_response_ms, None);
    assert_eq!(unmatched.tokens_per_second, None);
    fs::remove_dir_all(dir).unwrap();
}
