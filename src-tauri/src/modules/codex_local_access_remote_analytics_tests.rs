use super::*;

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    create_request_logs_table(&conn, true).unwrap();
    conn.execute_batch("CREATE INDEX idx_key_time ON request_logs(api_key_id,timestamp DESC);")
        .unwrap();
    conn
}

#[test]
fn image_logs_with_session_only_identity_are_visible_in_both_remote_scopes() {
    let conn=database();ensure_remote_analytics_schema(&conn).unwrap();
    conn.execute_batch("CREATE INDEX idx_codex_local_access_logs_request_id ON request_logs(request_id,timestamp DESC);").unwrap();
    create_request_diagnostics_table(&conn).unwrap();
    let thread="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    insert(&conn,1,"A",REMOTE_HOUR);
    insert(&conn,2,"A",REMOTE_HOUR+1);
    conn.execute("UPDATE request_logs SET requested_model='gpt-image-2.5',request_kind='image_generation',client_instance_id='instance-a',http_status=200",[]).unwrap();
    let attempts=json!([{"sequence":1,"accountId":"private-account","startedAtMs":REMOTE_HOUR,"status":200}]);
    conn.execute("INSERT INTO request_diagnostics(request_id,client_identity_captured,client_session_id,
        client_identity_metadata_captured,client_installation_id,captured_at_ms,attempts_json) VALUES('1',1,?1,1,'install-a',?2,?3)",
        params![thread,REMOTE_HOUR,attempts.to_string()]).unwrap();
    crate::modules::codex_request_summary::refresh_request(&conn,"1").unwrap();
    let query=crate::modules::codex_remote_service::KeyQuery{thread_id:Some(thread.into()),instance_id:Some("instance-a".into()),
        installation_id:Some("install-a".into()),..Default::default()};
    let chat=query_remote_key_requests(&conn,"A",0,2*REMOTE_HOUR,&query).unwrap();
    assert_eq!(chat["items"].as_array().unwrap().len(),1);
    assert_eq!(chat["items"][0]["model"],"gpt-image-2.5");
    assert_eq!(chat["items"][0]["kind"],"image_generation");
    let key=query_remote_key_requests(&conn,"A",0,2*REMOTE_HOUR,&Default::default()).unwrap();
    assert_eq!(key["items"].as_array().unwrap().len(),2);
    assert!(!chat.to_string().contains("private-account"));
}

#[test]
fn remote_session_uses_shared_metrics_and_exact_key_instance_installation_scope() {
    let conn=database();
    ensure_remote_analytics_schema(&conn).unwrap();
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    let thread="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    for (id,key,instance,installation,output,duration,input,cache) in [
        (1,"A","instance-a","install-a",200,2000,1000,500),
        (2,"A","instance-a","install-a",600,3000,3000,2700),
        (3,"B","instance-a","install-a",9999,1000,100,100),
        (4,"A","instance-b","install-a",9999,1000,100,100),
        (5,"A","instance-a","install-b",9999,1000,100,100),
    ] {
        insert(&conn,id,key,REMOTE_HOUR+id);
        conn.execute("UPDATE request_logs SET first_response_ms=300,output_generation_ms=?2,output_tokens=?3,input_tokens=?4,cached_tokens=?5 WHERE id=?1",params![id,duration,output,input,cache]).unwrap();
        let data=json!({"success":true,"requestedModel":"public-model","model":"secret-upstream",
            "requestedServiceTier":"priority","inputTokens":input,"cachedTokens":cache,
            "outputTokens":output,"outputGenerationMs":duration});
        conn.execute("INSERT INTO request_display_summaries(log_id,request_id,timestamp,api_key_id,client_instance_id,client_thread_id,
            client_installation_id,client_identity_captured,client_identity_metadata_captured,dispatched,data_json)
            VALUES(?1,?2,?3,?4,?5,?6,?7,1,1,1,?8)",params![id,id.to_string(),REMOTE_HOUR+id,key,instance,thread,installation,data.to_string()]).unwrap();
    }
    let query=crate::modules::codex_remote_service::KeyQuery{thread_id:Some(thread.into()),instance_id:Some("instance-a".into()),
        installation_id:Some("install-a".into()),model:Some("public-model".into()),service_tier:Some("priority".into()),..Default::default()};
    let result=query_remote_key_session(&conn,"A",&query).unwrap();
    assert_eq!(result["ready"],true);
    assert_eq!(result["session"]["firstResponseMs"],300);
    assert_eq!(result["metrics"]["samples"],2);
    assert_eq!(result["metrics"]["tokensPerSecond"],160.0);
    assert_eq!(result["metrics"]["averageCacheHitRate"],0.8);
    assert_eq!(result["metrics"]["selected"]["requestId"],"2");
    assert_eq!(result["metrics"]["selected"]["tokensPerSecond"],200.0);
    assert_eq!(result["metrics"],crate::modules::codex_app_injection::remote_key_session_metrics(&conn,"A",&query).unwrap());
    for secret in ["secret-upstream","secret@example.com","private-account","accountId"] {assert!(!result.to_string().contains(secret));}
    assert!(query_remote_key_session(&conn,"unknown",&query).unwrap()["session"].is_null());
    let page=query_remote_key_requests(&conn,"A",0,2*REMOTE_HOUR,&query).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(),2);
    assert_eq!(page["items"][0]["requestId"],"2");
    let mut page_query=query.clone();page_query.page_size=Some(1);
    let first=query_remote_key_requests(&conn,"A",0,2*REMOTE_HOUR,&page_query).unwrap();
    page_query.before_time=first["nextCursor"]["beforeTime"].as_i64();
    page_query.before_id=first["nextCursor"]["beforeId"].as_i64();
    let second=query_remote_key_requests(&conn,"A",0,2*REMOTE_HOUR,&page_query).unwrap();
    assert_eq!(second["items"][0]["requestId"],"1");
    assert!(second["nextCursor"].is_null());
    let plan=conn.prepare("EXPLAIN QUERY PLAN SELECT l.id FROM request_display_summaries s INDEXED BY idx_request_display_chat CROSS JOIN request_logs l ON l.id=s.log_id
        WHERE s.api_key_id='A' AND s.client_instance_id='instance-a' AND s.client_thread_id=?1 ORDER BY s.timestamp DESC,s.log_id DESC LIMIT 101").unwrap()
        .query_map([thread],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join("\n");
    assert!(plan.contains("SEARCH s USING COVERING INDEX idx_request_display_chat"),"{plan}");
    assert!(plan.contains("SEARCH l USING INTEGER PRIMARY KEY"),"{plan}");
    assert!(!plan.contains("TEMP B-TREE"),"{plan}");
}
fn insert(conn: &Connection, id: i64, key: &str, time: i64) {
    conn.execute("INSERT INTO request_logs(event_key,timestamp,request_id,api_key_id,requested_model,
        account_id,email,upstream_model,error_message,success,input_tokens,total_tokens,estimated_cost_usd)
        VALUES(?1,?2,?1,?3,'public-model','private-account','secret@example.com','secret-upstream','secret error',1,10,12,0.5)",
        params![id.to_string(),time,key]).unwrap();
}
fn finish(conn: &Connection) {
    while !remote_analytics_backfill_batch(conn).unwrap() {}
}

#[test]
fn background_backfill_recovers_and_tracks_live_writes_repricing_and_deletion() {
    let conn = database();
    for id in 1..=300 {
        insert(&conn, id, "A", REMOTE_HOUR);
    }
    ensure_remote_analytics_schema(&conn).unwrap();
    assert!(!remote_analytics_backfill_batch(&conn).unwrap());
    assert_eq!(
        conn.query_row("SELECT cursor FROM cockpit_key_analytics_state", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        crate::modules::background_maintenance::BATCH_ROWS as i64
    );
    // A crash/failing batch must not advance the checkpoint or partly count rows.
    conn.execute_batch("CREATE TRIGGER fail_batch BEFORE UPDATE ON cockpit_key_hourly_v2 BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(remote_analytics_backfill_batch(&conn).is_err());
    assert_eq!(
        conn.query_row("SELECT cursor FROM cockpit_key_analytics_state", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        crate::modules::background_maintenance::BATCH_ROWS as i64
    );
    conn.execute_batch("DROP TRIGGER fail_batch;").unwrap();
    insert(&conn, 301, "B", REMOTE_HOUR);
    conn.execute("DELETE FROM request_logs WHERE id=200", [])
        .unwrap();
    conn.execute(
        "UPDATE request_logs SET estimated_cost_usd=1 WHERE id IN (1,201)",
        [],
    )
    .unwrap();
    // Repeated schema initialization and continued batches must be idempotent.
    ensure_remote_analytics_schema(&conn).unwrap();
    finish(&conn);
    finish(&conn);
    let result = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(result["summary"]["requestCount"], 299);
    assert_eq!(result["summary"]["estimatedCostUsd"], 150.5);
    assert_eq!(
        query_remote_key_stats(&conn, "B", 0, 2 * REMOTE_HOUR).unwrap()["summary"]["requestCount"],
        1
    );
    conn.execute(
        "UPDATE request_logs SET settled_microusd=123,settlement_status='settled' WHERE id=1",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE request_logs SET estimated_cost_usd=900 WHERE id=1",
        [],
    )
    .unwrap();
    let result = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(result["summary"]["settledMicrousd"], 123);
    assert_eq!(result["summary"]["unrecordedCount"], 298);
    conn.execute("DELETE FROM request_logs", []).unwrap();
    assert_eq!(
        query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap()["summary"]["requestCount"],
        0
    );
}

#[test]
fn public_projection_is_exactly_key_scoped_and_cursor_does_not_skip_equal_timestamps() {
    let conn = database();
    ensure_remote_analytics_schema(&conn).unwrap();
    for id in 1..=5 {
        insert(&conn, id, if id == 3 { "B" } else { "A" }, REMOTE_HOUR);
    }
    let mut query = crate::modules::codex_remote_service::KeyQuery {
        page_size: Some(2),
        ..Default::default()
    };
    let first = query_remote_key_requests(&conn, "A", 0, 2 * REMOTE_HOUR, &query).unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    query.before_time = first
        .pointer("/nextCursor/beforeTime")
        .and_then(Value::as_i64);
    query.before_id = first
        .pointer("/nextCursor/beforeId")
        .and_then(Value::as_i64);
    let second = query_remote_key_requests(&conn, "A", 0, 2 * REMOTE_HOUR, &query).unwrap();
    assert_eq!(second["items"][0]["requestId"], "4");
    assert_eq!(second["items"][1]["requestId"], "5");
    assert!(second["nextCursor"].is_null());
    let serialized = first.to_string();
    for secret in [
        "private-account",
        "secret@example.com",
        "secret-upstream",
        "secret error",
        "accountId",
        "apiKeyId",
    ] {
        assert!(!serialized.contains(secret), "leaked {secret}");
    }
    assert_eq!(first["items"][0]["settlementStatus"], "unrecorded");
    query.before_id = None;
    query.before_time = None;
    query.model = Some("secret-upstream".into());
    assert!(
        query_remote_key_requests(&conn, "A", 0, 2 * REMOTE_HOUR, &query).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn large_history_queries_use_key_indexes_and_hourly_rollups() {
    let conn = database();
    ensure_remote_analytics_schema(&conn).unwrap();
    // 50,000 requests collapse to one hourly row, never to 50,000 Rust objects.
    conn.execute_batch("BEGIN;WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50000)
        INSERT INTO request_logs(event_key,timestamp,api_key_id,requested_model,success,total_tokens)
        SELECT CAST(x AS TEXT),3600000,'A','public-model',1,12 FROM n;COMMIT;").unwrap();
    let stats = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 50000);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM cockpit_key_hourly_v2", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    for (sql,expected) in [
        ("EXPLAIN QUERY PLAN SELECT * FROM cockpit_key_hourly_v2 WHERE generation=1 AND key_id='A' AND bucket>=0 AND bucket<7200000","PRIMARY KEY"),
        ("EXPLAIN QUERY PLAN SELECT id FROM request_logs WHERE api_key_id='A' AND timestamp>=0 AND timestamp<7200000 ORDER BY timestamp DESC,id ASC LIMIT 31","idx_key_time"),
    ] {
        let plan=conn.prepare(sql).unwrap().query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join(" ");
        assert!(plan.contains(expected),"{plan}");assert!(!plan.contains("SCAN "),"{plan}");
        assert!(!plan.contains("TEMP B-TREE"),"{plan}");
    }
}

#[test]
fn query_bounds_and_unknown_key_parameters_are_rejected() {
    let now = 100 * 86_400_000;
    assert!(remote_query_bounds(&Default::default(), now).is_ok());
    assert!(remote_query_bounds(
        &crate::modules::codex_remote_service::KeyQuery {
            start_at: Some(0),
            end_at: Some(now),
            ..Default::default()
        },
        now
    )
    .is_err());
    assert!(
        serde_urlencoded::from_str::<crate::modules::codex_remote_service::KeyQuery>("apiKeyId=B")
            .is_err()
    );
    assert!(
        serde_urlencoded::from_str::<crate::modules::codex_remote_service::KeyQuery>("startAt=abc")
            .is_err()
    );
}

#[test]
fn clear_during_backfill_restarts_generation_before_rowids_are_reused() {
    let conn = database();
    for id in 1..=100 {
        insert(&conn, id, "A", REMOTE_HOUR);
    }
    ensure_remote_analytics_schema(&conn).unwrap();
    assert!(!remote_analytics_backfill_batch(&conn).unwrap());
    let old_generation: i64 = conn
        .query_row(
            "SELECT generation FROM cockpit_key_analytics_state",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute("DELETE FROM request_logs", []).unwrap();
    // Simulate restoring a cleared database whose AUTOINCREMENT sequence was
    // reset. A normal clear retains the sequence; both cases must be safe.
    conn.execute("DELETE FROM sqlite_sequence WHERE name='request_logs'", [])
        .unwrap();
    insert(&conn, 1001, "A", REMOTE_HOUR);
    assert_eq!(
        conn.query_row("SELECT id FROM request_logs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let generation: i64 = conn
        .query_row(
            "SELECT generation FROM cockpit_key_analytics_state",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(generation > old_generation);
    finish(&conn);
    finish(&conn);
    let stats = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 1);
    assert_eq!(stats["summary"]["estimatedCostUsd"], 0.5);
    while !remote_analytics_cleanup_batch(&conn).unwrap() {}
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM cockpit_key_hourly_v2 WHERE generation<>?1",
            [generation],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn unversioned_upgrade_preserves_raw_and_previous_projection_until_new_generation_completes() {
    let conn = database();
    for id in 1..=80 {
        insert(&conn, id, "A", REMOTE_HOUR);
    }
    // Simulate the exact unversioned schema from the first local test build.
    conn.execute_batch("ALTER TABLE request_logs ADD COLUMN settled_microusd INTEGER;
        ALTER TABLE request_logs ADD COLUMN settlement_status TEXT NOT NULL DEFAULT 'unrecorded';
        ALTER TABLE request_logs ADD COLUMN settled_at INTEGER;
        CREATE TABLE cockpit_key_analytics_state(id INTEGER PRIMARY KEY,cursor INTEGER,high_water INTEGER);
        INSERT INTO cockpit_key_analytics_state VALUES(1,80,80);").unwrap();
    let definitions = remote_aggregate_columns("r")
        .iter()
        .map(|(name, _)| format!("{name} NUMERIC NOT NULL DEFAULT 0"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute_batch(&format!("CREATE TABLE cockpit_key_hourly(key_id TEXT,bucket INTEGER,{definitions},PRIMARY KEY(key_id,bucket)) WITHOUT ROWID;
        INSERT INTO cockpit_key_hourly(key_id,bucket,requests,total_tokens,estimated_usd,unrecorded) VALUES('A',3600000,80,960,40,80);")).unwrap();
    ensure_remote_analytics_schema(&conn).unwrap();
    assert_eq!(
        query_remote_key_stats_projection(&conn, "A", 0, 2 * REMOTE_HOUR, true).unwrap()["summary"]
            ["requestCount"],
        80
    );
    assert!(!remote_analytics_backfill_batch(&conn).unwrap());
    // Repeat initialization mid-upgrade; the durable checkpoint must survive.
    ensure_remote_analytics_schema(&conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT cursor FROM cockpit_key_analytics_state", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        32
    );
    insert(&conn, 81, "A", REMOTE_HOUR);
    conn.execute(
        "UPDATE request_logs SET estimated_cost_usd=1 WHERE id=70",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM request_logs WHERE id=5", [])
        .unwrap();
    finish(&conn);
    let stats = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 80);
    assert_eq!(stats["summary"]["estimatedCostUsd"], 40.5);
    assert_eq!(
        conn.query_row("SELECT version FROM cockpit_key_analytics_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        REMOTE_ANALYTICS_VERSION
    );
    while !remote_analytics_cleanup_batch(&conn).unwrap() {}
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        80
    );
    assert_eq!(
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cockpit_key_hourly')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        false
    );
}

#[test]
fn external_database_lock_rolls_back_batch_without_advancing_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lock.sqlite");
    let conn = Connection::open(&path).unwrap();
    create_request_logs_table(&conn, true).unwrap();
    conn.busy_timeout(Duration::from_millis(5)).unwrap();
    for id in 1..=64 {
        insert(&conn, id, "A", REMOTE_HOUR);
    }
    ensure_remote_analytics_schema(&conn).unwrap();
    let other = Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    assert!(remote_analytics_backfill_batch(&conn).is_err());
    assert!(started.elapsed() < Duration::from_millis(250));
    assert_eq!(
        conn.query_row("SELECT cursor FROM cockpit_key_analytics_state", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    other.execute_batch("ROLLBACK").unwrap();
    finish(&conn);
    assert_eq!(
        query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap()["summary"]["requestCount"],
        64
    );
}

#[test]
fn oversized_history_does_not_enter_rust_projection_or_block_following_rows() {
    let conn = database();
    insert(&conn, 1, &"x".repeat(4 * 1024 * 1024), REMOTE_HOUR);
    insert(&conn, 2, "A", REMOTE_HOUR);
    conn.execute(
        "UPDATE request_logs SET error_message=?1 WHERE id=2",
        ["x".repeat(4 * 1024 * 1024)],
    )
    .unwrap();
    finish(&conn);
    assert_eq!(
        conn.query_row(
            "SELECT oversized FROM cockpit_key_analytics_state",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let stats = query_remote_key_stats(&conn, "A", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 1);
    assert!(
        crate::modules::background_maintenance::BATCH_ROWS * REMOTE_PROJECTION_ROW_BYTES
            <= crate::modules::background_maintenance::BATCH_BYTES as usize
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn reopening_database_resumes_durable_checkpoint_and_completed_migration_does_no_work() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("restart.sqlite");
    {
        let conn=Connection::open(&path).unwrap();create_request_logs_table(&conn,true).unwrap();
        for id in 1..=70 {insert(&conn,id,"A",REMOTE_HOUR);}
        assert!(!remote_analytics_backfill_batch(&conn).unwrap());
        assert_eq!(conn.query_row("SELECT cursor FROM cockpit_key_analytics_state",[],|r|r.get::<_,i64>(0)).unwrap(),32);
    }
    let conn=Connection::open(&path).unwrap();finish(&conn);
    let changes=conn.total_changes();
    assert!(remote_analytics_backfill_batch(&conn).unwrap());
    assert_eq!(conn.total_changes(),changes);
    assert_eq!(query_remote_key_stats(&conn,"A",0,2*REMOTE_HOUR).unwrap()["summary"]["requestCount"],70);
}
