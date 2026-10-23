use super::*;

const THREAD: &str = "019e92aa-0000-7000-8000-000000000001";
const OTHER_THREAD: &str = "019e92aa-0000-7000-8000-000000000002";

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE request_logs(
         id INTEGER PRIMARY KEY,timestamp INTEGER,request_id TEXT,account_id TEXT,email TEXT,
         api_key_id TEXT,client_instance_id TEXT,model_id TEXT,requested_model TEXT,
         upstream_model TEXT,reasoning_effort TEXT,authorization_route TEXT,success INTEGER,
         http_status INTEGER,error_category TEXT,error_message TEXT,latency_ms INTEGER,
         first_response_ms INTEGER,output_generation_ms INTEGER,input_tokens INTEGER,
         output_tokens INTEGER,total_tokens INTEGER,cached_tokens INTEGER,
         requested_service_tier TEXT,response_service_tier TEXT,service_tier TEXT);
         CREATE INDEX idx_codex_local_access_logs_request_id ON request_logs(request_id,timestamp DESC);
         CREATE TABLE request_diagnostics(request_id TEXT PRIMARY KEY,
         client_identity_captured INTEGER,client_thread_id TEXT,
         client_identity_metadata_captured INTEGER,client_installation_id TEXT,
         attempts_json TEXT,failure_phase TEXT,payloads_json TEXT);",
    ).unwrap();
    conn
}

fn log(conn: &Connection, request: &str, timestamp: i64) {
    conn.execute(
        "INSERT INTO request_logs(timestamp,request_id,account_id,email,api_key_id,
         client_instance_id,model_id,requested_model,upstream_model,reasoning_effort,
         authorization_route,success,http_status,error_category,error_message,latency_ms,
         first_response_ms,output_generation_ms,input_tokens,output_tokens,total_tokens,
         cached_tokens,requested_service_tier,response_service_tier,service_tier)
         VALUES(?1,?2,'original','original@example.test','key-a','instance-a','model-a',
         'model-a','model-a','high','official',1,200,'','SECRET_ERROR',2000,500,1500,
         1000,300,1300,800,'priority','default','priority')",
        params![timestamp, request],
    )
    .unwrap();
}

fn attempt(account: &str, sequence: u32, route: &str) -> Value {
    json!({"sequence":sequence,"accountId":account,
        "accountEmail":format!("{account}@example.test"),
        "authorizationRoute":route,"transport":"http","status":200,
        "startedAtMs":1000+sequence,"errorMessage":"SECRET_ATTEMPT_ERROR"})
}

fn detail(conn: &Connection, request: &str, thread: &str, attempts: Value) {
    conn.execute(
        "INSERT OR REPLACE INTO request_diagnostics VALUES(?1,1,?2,1,'installation-a',?3,
         NULL,'SECRET_REQUEST_RESPONSE_BODY')",
        params![request, thread, attempts.to_string()],
    )
    .unwrap();
}

fn summary(conn: &Connection, request: &str) -> Value {
    let encoded: String = conn.query_row(
        "SELECT data_json FROM request_display_summaries WHERE request_id=?1 ORDER BY log_id DESC",
        [request], |row| row.get(0),
    ).unwrap();
    serde_json::from_str(&encoded).unwrap()
}

fn count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM request_display_summaries",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn empty_projection_schema_never_scans_or_backfills_history() {
    let conn = database();
    for index in 0..100 {
        log(&conn, &format!("historical-{index}"), index);
    }
    ensure_schema(&conn).unwrap();
    assert_eq!(count(&conn), 0);
    log(&conn, "live", 101);
    refresh_request(&conn, "live").unwrap();
    assert_eq!(count(&conn), 1);
    assert_eq!(summary(&conn, "live")["timestamp"], 101);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        101
    );
}

#[test]
fn usage_and_detail_order_are_idempotent_and_identity_is_canonical() {
    for diagnostic_first in [true, false] {
        let conn = database();
        let compact_uppercase = THREAD.replace('-', "").to_uppercase();
        if diagnostic_first {
            detail(
                &conn,
                "live",
                &compact_uppercase,
                json!([attempt("final", 1, "official")]),
            );
            refresh_request(&conn, "live").unwrap();
            assert_eq!(count(&conn), 0);
        }
        log(&conn, "live", 100);
        refresh_request(&conn, "live").unwrap();
        if !diagnostic_first {
            let flags: (bool, Option<String>) = conn.query_row(
                "SELECT client_identity_captured,client_thread_id FROM request_display_summaries",
                [], |row| Ok((row.get(0)?,row.get(1)?)),
            ).unwrap();
            assert_eq!(flags, (false, None));
            detail(
                &conn,
                "live",
                &compact_uppercase,
                json!([attempt("final", 1, "official")]),
            );
            refresh_request(&conn, "live").unwrap();
        }
        refresh_request(&conn, "live").unwrap();
        assert_eq!(count(&conn), 1);
        let flags: (bool, bool, String, String) = conn
            .query_row(
                "SELECT client_identity_captured,client_identity_metadata_captured,
             client_thread_id,client_installation_id FROM request_display_summaries",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(flags, (true, true, THREAD.into(), "installation-a".into()));
        assert_eq!(summary(&conn, "live")["accountId"], "final");
    }
}

#[test]
fn exact_chat_index_keeps_keys_instances_threads_and_installations_isolated() {
    let conn = database();
    for (request, thread) in [
        ("same", THREAD),
        ("other-thread", OTHER_THREAD),
        ("other-key", THREAD),
        ("other-instance", THREAD),
        ("other-installation", THREAD),
        ("invalid-thread", "not-a-uuid"),
    ] {
        log(&conn, request, 100);
        detail(
            &conn,
            request,
            thread,
            json!([attempt("account", 1, "local_gateway")]),
        );
    }
    conn.execute(
        "UPDATE request_logs SET api_key_id='key-b' WHERE request_id='other-key'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE request_logs SET client_instance_id='instance-b' WHERE request_id='other-instance'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE request_diagnostics SET client_installation_id='installation-b' WHERE request_id='other-installation'", []).unwrap();
    for request in [
        "same",
        "other-thread",
        "other-key",
        "other-instance",
        "other-installation",
        "invalid-thread",
    ] {
        refresh_request(&conn, request).unwrap();
    }
    let rows: Vec<String> = conn
        .prepare(
            "SELECT request_id FROM request_display_summaries INDEXED BY idx_request_display_chat
         WHERE api_key_id='key-a' AND client_instance_id='instance-a' AND client_thread_id=?1
         AND client_identity_captured=1 AND client_identity_metadata_captured=1
         AND client_installation_id='installation-a' AND dispatched=1
         ORDER BY timestamp DESC,log_id DESC LIMIT 10",
        )
        .unwrap()
        .query_map([THREAD], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, ["same"]);
    let invalid: (Option<String>,bool) = conn.query_row(
        "SELECT client_thread_id,client_identity_captured FROM request_display_summaries WHERE request_id='invalid-thread'",
        [], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(invalid, (None, false));
}

#[test]
fn preflight_is_never_shown_as_dispatched_and_invalid_diagnostics_are_not_guessed() {
    let conn = database();
    for (request, attempts) in [
        (
            "preflight",
            json!([{"sequence":1,"transport":"preflight",
        "accountId":"selected","status":200,"failurePhase":"preparation"}]),
        ),
        ("no-attempt", json!([])),
        (
            "failed-upstream",
            json!([{
            "sequence":1,"accountId":"selected","transport":"http","status":403}]),
        ),
    ] {
        log(&conn, request, 100);
        detail(&conn, request, THREAD, attempts);
        refresh_request(&conn, request).unwrap();
    }
    let rows: Vec<String> = conn
        .prepare("SELECT request_id FROM request_display_summaries WHERE dispatched=1")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, ["failed-upstream"]);
    conn.execute("UPDATE request_diagnostics SET attempts_json='malformed' WHERE request_id='failed-upstream'", []).unwrap();
    refresh_request(&conn, "failed-upstream").unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT dispatched FROM request_display_summaries WHERE request_id='failed-upstream'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn final_attempt_account_route_fallback_and_tier_evidence_match_api_service() {
    let conn = database();
    log(&conn, "live", 100);
    let mut official = attempt("final", 1, "official");
    official["status"] = json!(403);
    official["serviceTierAdaptation"] = json!({
        "requestedTier":"priority","sentTier":"default","reason":"official_default_only"});
    detail(&conn, "live", THREAD, json!([official.clone()]));
    refresh_request(&conn, "live").unwrap();
    let first = summary(&conn, "live");
    assert_eq!(first["requestedServiceTier"], "priority");
    assert_eq!(first["responseServiceTier"], "default");
    assert_eq!(first["serviceTierAdaptation"]["requestedTier"], "priority");
    let mut local = attempt("final", 2, "local_gateway");
    local["authorizationFallback"] = json!({"fromRoute":"official","toRoute":"local_gateway",
        "fromAccountId":"final","toAccountId":"final","reason":"eligibility_rejected"});
    detail(&conn, "live", THREAD, json!([local.clone(), official]));
    refresh_request(&conn, "live").unwrap();
    let final_row = summary(&conn, "live");
    assert_eq!(final_row["accountId"], "final");
    assert_eq!(final_row["accountEmail"], "final@example.test");
    assert_eq!(final_row["authorizationRoute"], "local_gateway");
    assert_eq!(final_row["recovered"], true);
    assert!(final_row["serviceTierAdaptation"].is_null());
    let encoded = final_row.to_string();
    for forbidden in [
        "SECRET",
        "attempts",
        "payloads",
        "authorizationFallback",
        "original@example.test",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    assert!(final_row["plan"].is_null());
    assert!(final_row["errorMessage"].is_null());
}

#[test]
fn tier_adaptation_rejects_unproven_or_invalid_final_route_metadata() {
    let mut official = attempt("account", 1, "official");
    official["serviceTierAdaptation"] = json!({
        "requestedTier":"priority","sentTier":"default","reason":"official_default_only"});
    assert!(service_tier_adaptation(&json!([official.clone()]).to_string()).is_some());
    for (field, value) in [
        ("sentTier", "priority"),
        ("requestedTier", "auto"),
        ("requestedTier", "default"),
        ("requestedTier", "secret.field"),
        ("reason", "guessed"),
    ] {
        let mut invalid = official.clone();
        invalid["serviceTierAdaptation"][field] = json!(value);
        assert!(service_tier_adaptation(&json!([invalid]).to_string()).is_none());
    }
    assert!(service_tier_adaptation(
        &json!([official, attempt("account", 2, "local_gateway")]).to_string()
    )
    .is_none());
    assert!(service_tier_adaptation("[]").is_none());
    assert!(service_tier_adaptation(&"x".repeat(MAX_ATTEMPTS_BYTES + 1)).is_none());
}

#[test]
fn multiple_usage_rows_keep_distinct_ids_and_original_row_metrics() {
    let conn = database();
    log(&conn, "shared-request", 100);
    log(&conn, "shared-request", 101);
    conn.execute(
        "UPDATE request_logs SET output_tokens=600 WHERE timestamp=101",
        [],
    )
    .unwrap();
    detail(
        &conn,
        "shared-request",
        THREAD,
        json!([attempt("final", 1, "official")]),
    );
    refresh_request(&conn, "shared-request").unwrap();
    assert_eq!(count(&conn), 2);
    let rows: Vec<String> = conn
        .prepare("SELECT data_json FROM request_display_summaries ORDER BY log_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let first: Value = serde_json::from_str(&rows[0]).unwrap();
    let second: Value = serde_json::from_str(&rows[1]).unwrap();
    assert_ne!(first["id"], second["id"]);
    assert_eq!(first["outputTokens"], 300);
    assert_eq!(second["outputTokens"], 600);
}

#[test]
fn diagnostic_retention_and_usage_replay_preserve_captured_identity_and_final_account() {
    let conn = database();
    log(&conn, "live", 100);
    detail(
        &conn,
        "live",
        THREAD,
        json!([attempt("final", 1, "local_gateway")]),
    );
    refresh_request(&conn, "live").unwrap();
    let before = summary(&conn, "live");
    conn.execute("DELETE FROM request_diagnostics", []).unwrap();
    refresh_request(&conn, "live").unwrap();
    assert_eq!(summary(&conn, "live"), before);
    assert_eq!(
        conn.query_row(
            "SELECT client_thread_id FROM request_display_summaries",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        THREAD
    );
}

#[test]
fn projection_failure_does_not_destroy_logs_or_partial_overwrite_and_can_retry() {
    let conn = database();
    log(&conn, "live", 100);
    log(&conn, "live", 101);
    ensure_schema(&conn).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_second_summary BEFORE INSERT ON request_display_summaries
        WHEN NEW.timestamp=100 BEGIN SELECT RAISE(FAIL,'simulated summary failure'); END;",
    )
    .unwrap();
    assert!(refresh_request(&conn, "live").is_err());
    assert_eq!(count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    conn.execute("DROP TRIGGER fail_second_summary", [])
        .unwrap();
    refresh_request(&conn, "live").unwrap();
    assert_eq!(count(&conn), 2);
}

#[test]
fn clearing_is_transactional_and_does_not_create_missing_projection_schema() {
    let conn = database();
    clear_if_present(&conn).unwrap();
    assert!(!table_exists(&conn, "request_display_summaries").unwrap());
    log(&conn, "live", 100);
    refresh_request(&conn, "live").unwrap();
    conn.execute_batch("BEGIN").unwrap();
    clear_if_present(&conn).unwrap();
    assert_eq!(count(&conn), 0);
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(count(&conn), 1);
    conn.execute_batch("BEGIN").unwrap();
    clear_if_present(&conn).unwrap();
    conn.execute("DELETE FROM request_logs", []).unwrap();
    conn.execute_batch("COMMIT").unwrap();
    refresh_request(&conn, "live").unwrap();
    assert_eq!(count(&conn), 0);
}

#[test]
fn direct_chat_and_service_queries_use_summary_indexes_without_join_or_sort_scan() {
    let conn = database();
    ensure_schema(&conn).unwrap();
    for (sql, expected) in [
        ("SELECT data_json FROM request_display_summaries WHERE api_key_id='key-a'
          AND client_instance_id='instance-a' AND client_thread_id='thread'
          AND client_identity_captured=1 AND dispatched=1 ORDER BY timestamp DESC,log_id DESC LIMIT 10",
         "idx_request_display_chat"),
        ("SELECT data_json FROM request_display_summaries WHERE dispatched=1
          ORDER BY timestamp DESC,log_id DESC LIMIT 10", "idx_request_display_timestamp"),
    ] {
        let plan: Vec<String> = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap().query_map([], |row| row.get(3)).unwrap()
            .collect::<Result<_, _>>().unwrap();
        assert!(plan.iter().any(|step| step.contains(expected)), "{plan:?}");
        assert!(plan.iter().all(|step| !step.contains("TEMP B-TREE")), "{plan:?}");
        assert!(plan.iter().all(|step| !step.contains("request_logs") && !step.contains("request_diagnostics")));
    }
}

#[test]
fn display_projection_is_bounded_and_never_serializes_raw_attempts_or_errors() {
    let conn = database();
    log(&conn, "bounded", 100);
    let worst_case = "\\\"".repeat(128);
    conn.execute(
        "UPDATE request_logs SET model_id=?1,requested_model=?1,upstream_model=?1,
         reasoning_effort=?1,requested_service_tier=?1,response_service_tier=?1,
         service_tier=?1,error_message=?2 WHERE request_id='bounded'",
        params![worst_case, "SECRET_ERROR".repeat(10_000)],
    )
    .unwrap();
    detail(
        &conn,
        "bounded",
        THREAD,
        json!([attempt("final", 1, "official")]),
    );
    refresh_request(&conn, "bounded").unwrap();
    let data = summary(&conn, "bounded");
    let encoded = encode_display_data(&data).unwrap();
    assert!(encoded.len() <= MAX_DISPLAY_BYTES);
    assert!(!encoded.contains("SECRET"));
    assert!(data["errorMessage"].is_null());
    assert_eq!(data["inputTokens"], 1000);
    assert_eq!(data["cachedTokens"], 800);
    assert!(
        encode_display_data(&json!({"unexpectedField":"x".repeat(MAX_DISPLAY_BYTES)})).is_err()
    );
}
