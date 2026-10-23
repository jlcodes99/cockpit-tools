use super::*;

const THREAD: &str = "019e92aa-0000-7000-8000-000000000001";
const OTHER_THREAD: &str = "019e92aa-0000-7000-8000-000000000002";

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE request_logs (
         id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, request_id TEXT NOT NULL,
         account_id TEXT, email TEXT, api_key_id TEXT, client_instance_id TEXT,
         model_id TEXT,reasoning_effort TEXT,authorization_route TEXT,
         success INTEGER, http_status INTEGER,error_category TEXT,error_message TEXT,
         latency_ms INTEGER,first_response_ms INTEGER,output_generation_ms INTEGER,
         input_tokens INTEGER,output_tokens INTEGER,total_tokens INTEGER,
         cached_tokens INTEGER,reasoning_tokens INTEGER,estimated_cost_usd REAL);
         CREATE INDEX idx_codex_local_access_logs_timestamp ON request_logs(timestamp DESC);
         CREATE INDEX idx_codex_local_access_logs_api_key ON request_logs(api_key_id,timestamp DESC);
         CREATE TABLE request_diagnostics (
         request_id TEXT PRIMARY KEY,client_identity_captured INTEGER,client_thread_id TEXT,
         client_identity_metadata_captured INTEGER,client_installation_id TEXT,
         attempts_json TEXT,payloads_json TEXT,failure_phase TEXT);",
    ).unwrap();
    conn
}

fn query(scope: &str, view: &str) -> DialogQuery {
    DialogQuery {
        request_token: "dialog-1:1".into(),
        thread_id: Some(THREAD.into()),
        scope: scope.into(),
        period: "today".into(),
        cursor: None,
        view: view.into(),
        binding: None,
    }
}

fn scope() -> RequestScope {
    RequestScope {
        key_id: "key-a".into(),
        instance_id: Some("instance-a".into()),
        installation_id: Some("installation-a".into()),
    }
}

fn attempt(account: &str, status: u16) -> Value {
    json!({"sequence":1,"accountId":account,"accountEmail":format!("{account}@example.test"),
        "transport":"http","authorizationRoute":"official","startedAtMs":1000,"status":status})
}

fn append(conn: &Connection, request: &str, key: &str, thread: &str, timestamp: i64) {
    conn.execute(
        "INSERT INTO request_logs(timestamp,request_id,account_id,email,api_key_id,
            client_instance_id,model_id,reasoning_effort,authorization_route,
            success,http_status,error_category,error_message,latency_ms,first_response_ms,
            output_generation_ms,input_tokens,output_tokens,total_tokens,cached_tokens,
            reasoning_tokens,estimated_cost_usd)
         VALUES(?1,?2,'account-a','account-a@example.test',?3,'instance-a','model-a','high',
            'official',1,200,'','SECRET_ERROR',2000,500,1500,1000,300,1300,800,30,0.25)",
        params![timestamp, request, key],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO request_diagnostics VALUES(?1,1,?2,1,'installation-a',?3,
            'SECRET REQUEST RESPONSE AUTHORIZATION TOOLS',NULL)",
        params![
            request,
            thread,
            json!([attempt("account-a", 200)]).to_string()
        ],
    )
    .unwrap();
}

fn data(conn: &Connection, scope: &RequestScope, query: &DialogQuery) -> Value {
    with_budget(conn, QUERY_BUDGET, |started| {
        read_data(conn, scope, query, started, None)
    })
    .unwrap()
}

#[test]
fn query_validation_rejects_unbounded_or_ambiguous_input() {
    let valid = serde_json::to_value(query("chat", "overview")).unwrap();
    assert!(parse_query(&valid).is_some());
    for (key, bad) in [
        ("requestToken", json!("x".repeat(129))),
        ("requestToken", json!("line\nbreak")),
        ("threadId", json!("not-a-uuid")),
        ("scope", json!("all")),
        ("period", json!("year")),
        ("view", json!("payload")),
        ("cursor", json!("invalid")),
        ("additional", json!(true)),
    ] {
        let mut input = valid.clone();
        input[key] = bad;
        assert!(parse_query(&input).is_none(), "{key}");
    }
    let mut missing = valid.clone();
    missing["threadId"] = Value::Null;
    assert!(parse_query(&missing).is_some());
}

#[test]
fn session_identity_requires_exact_key_instance_thread_and_explicit_installation() {
    let conn = database();
    append(&conn, "correct", "key-a", THREAD, 1000);
    append(&conn, "key", "key-b", THREAD, 2000);
    append(&conn, "thread", "key-a", OTHER_THREAD, 3000);
    append(&conn, "instance", "key-a", THREAD, 4000);
    append(&conn, "installation", "key-a", THREAD, 5000);
    conn.execute(
        "UPDATE request_logs SET client_instance_id='other' WHERE request_id='instance'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE request_diagnostics SET client_installation_id='other' WHERE request_id='installation'", []).unwrap();
    let result = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(result["requests"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["requests"]["rows"][0]["requestId"], "correct");
    assert!(result["session"].is_null());
    assert_eq!(
        data(&conn, &scope(), &query("chat", "overview"))["session"]["threadId"],
        THREAD
    );
    conn.execute(
        "UPDATE request_diagnostics SET client_identity_captured=0 WHERE request_id='correct'",
        [],
    )
    .unwrap();
    assert!(data(&conn, &scope(), &query("chat", "overview"))["session"].is_null());
}

#[test]
fn absence_of_configured_instance_or_thread_never_guesses_session() {
    let conn = database();
    append(&conn, "one", "key-a", THREAD, 1000);
    let mut missing_scope = scope();
    missing_scope.instance_id = None;
    assert_eq!(
        data(&conn, &missing_scope, &query("chat", "logs"))["available"],
        false
    );
    let mut missing_thread = query("chat", "logs");
    missing_thread.thread_id = None;
    assert_eq!(data(&conn, &scope(), &missing_thread)["available"], false);
    missing_thread.scope = "service".into();
    let result = data(&conn, &scope(), &missing_thread);
    assert_eq!(result["requests"]["rows"].as_array().unwrap().len(), 1);
    assert!(result["session"].is_null());
    missing_thread.scope = "chat".into();
    missing_thread.view = "overview".into();
    let overview = data(&conn, &scope(), &missing_thread);
    assert_eq!(overview["available"], true);
    assert!(overview["summary"].is_object());
    assert!(overview["session"].is_null());
}

#[test]
fn installation_is_checked_only_when_explicitly_configured() {
    let conn = database();
    append(&conn, "one", "key-a", THREAD, 1000);
    conn.execute("UPDATE request_diagnostics SET client_identity_metadata_captured=0,client_installation_id=NULL", []).unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"],
        json!([])
    );
    let mut expected = scope();
    expected.installation_id = None;
    assert_eq!(
        data(&conn, &expected, &query("chat", "logs"))["requests"]["rows"],
        json!([])
    );
    conn.execute(
        "UPDATE request_diagnostics SET client_identity_metadata_captured=1",
        [],
    )
    .unwrap();
    assert_eq!(
        data(&conn, &expected, &query("chat", "logs"))["requests"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn service_spans_keys_but_session_metadata_stays_with_current_chat() {
    let conn = database();
    append(&conn, "mine", "key-a", THREAD, 1000);
    append(&conn, "other", "key-b", THREAD, 2000);
    conn.execute("UPDATE request_logs SET model_id='other-model',first_response_ms=900 WHERE request_id='other'", []).unwrap();
    let result = data(&conn, &scope(), &query("service", "logs"));
    assert_eq!(result["requests"]["rows"].as_array().unwrap().len(), 2);
    assert!(result["session"].is_null());
    let overview = data(&conn, &scope(), &query("service", "overview"));
    assert_eq!(overview["session"]["model"], "model-a");
    assert_eq!(overview["session"]["firstResponseMs"], 500);
}

#[test]
fn pagination_ties_and_concurrent_inserts_never_repeat_or_skip_original_rows() {
    let conn = database();
    for i in 0..23 {
        append(&conn, &format!("r-{i}"), "key-a", THREAD, 1000);
    }
    let mut request = query("chat", "logs");
    let first = data(&conn, &scope(), &request);
    assert_eq!(first["requests"]["rows"].as_array().unwrap().len(), 10);
    let mut ids = first["requests"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    // Both newer and backdated inserts are excluded from the existing snapshot.
    append(&conn, "new", "key-a", THREAD, 2000);
    append(&conn, "backdated", "key-a", THREAD, 500);
    request.cursor = first["requests"]["nextCursor"].as_str().map(str::to_owned);
    while request.cursor.is_some() {
        let result = data(&conn, &scope(), &request);
        ids.extend(
            result["requests"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string()),
        );
        request.cursor = result["requests"]["nextCursor"].as_str().map(str::to_owned);
    }
    assert_eq!(ids.len(), 23);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 23);
    assert_eq!(ids.first().unwrap(), "23");
    assert_eq!(ids.last().unwrap(), "1");
}

#[test]
fn sparse_session_scans_past_many_unrelated_rows_without_false_end() {
    let conn = database();
    for i in 0..12 {
        append(&conn, &format!("mine-{i}"), "key-a", THREAD, i);
    }
    for i in 0..1500 {
        append(
            &conn,
            &format!("other-{i}"),
            "key-a",
            OTHER_THREAD,
            1000 + i,
        );
    }
    let first = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(first["requests"]["rows"].as_array().unwrap().len(), 10);
    assert!(first["requests"]["nextCursor"].is_string());
    let mut next = query("chat", "logs");
    next.cursor = first["requests"]["nextCursor"].as_str().map(str::to_string);
    let last = data(&conn, &scope(), &next);
    assert_eq!(last["requests"]["rows"].as_array().unwrap().len(), 2);
    assert!(last["requests"]["nextCursor"].is_null());
    next.scope = "service".into();
    assert!(read_data(&conn, &scope(), &next, Instant::now(), None).is_err());
}

#[test]
fn upstream_failed_and_recovered_requests_are_kept_but_preflight_is_not() {
    let conn = database();
    for request in ["failed", "recovered", "preflight", "selected"] {
        append(&conn, request, "key-a", THREAD, 1000);
    }
    let mut preflight = attempt("never-sent", 503);
    preflight["transport"] = json!("preflight");
    preflight["failurePhase"] = json!("routing");
    let mut selected = attempt("never-sent", 503);
    selected.as_object_mut().unwrap().remove("status");
    let mut recovered = attempt("second", 200);
    recovered["sequence"] = json!(2);
    for (id, attempts) in [
        ("failed", json!([attempt("account-a", 429)])),
        (
            "recovered",
            json!([attempt("account-a", 429), recovered, preflight.clone()]),
        ),
        ("preflight", json!([preflight])),
        ("selected", json!([selected])),
    ] {
        conn.execute(
            "UPDATE request_diagnostics SET attempts_json=?1 WHERE request_id=?2",
            params![attempts.to_string(), id],
        )
        .unwrap();
    }
    conn.execute("UPDATE request_logs SET success=0,http_status=429,error_category='quota_exhausted' WHERE request_id='failed'",[]).unwrap();
    let result = data(&conn, &scope(), &query("chat", "logs"));
    let rows = result["requests"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["accountId"], "second");
    assert_eq!(rows[0]["recovered"], false);
    assert_eq!(rows[1]["success"], false);
}

#[test]
fn summary_uses_entire_time_window_cost_snapshot_and_success_only_latency() {
    let conn = database();
    for i in 0..325 {
        append(
            &conn,
            &format!("r-{i}"),
            if i % 2 == 0 { "key-a" } else { "key-b" },
            THREAD,
            1000 + i,
        );
    }
    append(&conn, "old", "key-a", THREAD, 999);
    append(&conn, "future", "key-a", THREAD, 2001);
    conn.execute(
        "UPDATE request_logs SET success=0,latency_ms=100000 WHERE request_id='r-0'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json='[]' WHERE request_id='r-1'",
        [],
    )
    .unwrap();
    let result = read_summary(&conn, 1000, 2000, Instant::now()).unwrap();
    assert_eq!(result["requests"], 324);
    assert_eq!(result["successRequests"], 323);
    assert_eq!(result["totalTokens"], 324 * 1300);
    assert_eq!(result["estimatedCostUsd"], 81.0);
    assert_eq!(result["averageLatencyMs"], 2000.0);
    let empty = read_summary(&conn, 3000, 4000, Instant::now()).unwrap();
    assert_eq!(empty["requests"], 0);
    assert!(empty["averageLatencyMs"].is_null());
}

#[test]
fn historical_responses_need_positive_response_evidence_and_unknowns_are_visible() {
    let conn = database();
    for id in ["historical", "unknown", "preflight", "zero-tokens"] {
        append(&conn, id, "key-a", THREAD, 1000);
    }
    conn.execute(
        "DELETE FROM request_diagnostics WHERE request_id<>'preflight'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE request_logs SET success=0,http_status=503 WHERE request_id='unknown'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE request_logs SET input_tokens=0,output_tokens=0,total_tokens=0 WHERE request_id='zero-tokens'", []).unwrap();
    let mut preflight = attempt("not-sent", 200);
    preflight["transport"] = json!("preflight");
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1 WHERE request_id='preflight'",
        [json!([preflight]).to_string()],
    )
    .unwrap();
    let summary = read_summary(&conn, 0, 2000, Instant::now()).unwrap();
    assert_eq!(summary["requests"], 1);
    assert_eq!(summary["excludedUnknownRequests"], 2);
    assert_eq!(summary["incomplete"], true);
    let service = data(&conn, &scope(), &query("service", "logs"));
    assert_eq!(service["requests"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(service["requests"]["rows"][0]["requestId"], "historical");
    assert_eq!(
        service["requests"]["rows"][0]["accountEmail"],
        "account-a@example.test"
    );
    // A response without captured identity cannot be linked to this conversation.
    assert!(service["session"].is_null());
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"],
        json!([])
    );
    append(&conn, "empty-routing", "key-a", THREAD, 1000);
    conn.execute("UPDATE request_diagnostics SET attempts_json='[]',failure_phase='routing' WHERE request_id='empty-routing'",[]).unwrap();
    let summary = read_summary(&conn, 0, 2000, Instant::now()).unwrap();
    assert_eq!(summary["requests"], 1);
    assert_eq!(summary["excludedUnknownRequests"], 2);
}

#[test]
fn unknown_timing_remains_null_and_no_payload_or_raw_error_is_returned() {
    let conn = database();
    append(&conn, "one", "key-a", THREAD, 1000);
    conn.execute(
        "UPDATE request_logs SET first_response_ms=NULL,output_generation_ms=NULL,latency_ms=0",
        [],
    )
    .unwrap();
    // Reject access even to reading the body column's size; only the diagnostic
    // attempt metadata is part of the projection.
    let result = data(&conn, &scope(), &query("chat", "logs"));
    let row = &result["requests"]["rows"][0];
    assert!(row["firstResponseMs"].is_null());
    assert!(row["outputGenerationMs"].is_null());
    assert_eq!(row["latencyMs"], 0);
    assert!(row["plan"].is_null());
    assert!(!result.to_string().contains("SECRET"));
    let source = include_str!("codex_app_injection_dialog_data.rs");
    assert!(!source.contains("d.payloads_json"));
    assert!(!source.contains("l.error_message"));
}

#[test]
fn oversized_attempts_and_old_schema_fail_closed_without_migration() {
    let conn = database();
    append(&conn, "one", "key-a", THREAD, 1000);
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1",
        [format!("[{}]", " ".repeat(65536))],
    )
    .unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"],
        json!([])
    );
    conn.execute(
        "ALTER TABLE request_logs DROP COLUMN output_generation_ms",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1",
        [json!([attempt("account-a", 200)]).to_string()],
    )
    .unwrap();
    assert!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"][0]["outputGenerationMs"]
            .is_null()
    );
    conn.execute(
        "ALTER TABLE request_diagnostics DROP COLUMN client_identity_captured",
        [],
    )
    .unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["available"],
        false
    );
}

#[test]
fn sqlite_execution_is_interrupted_instead_of_returning_fake_empty_data() {
    let conn = database();
    let result = with_budget(&conn, Duration::from_millis(20), |_| {
        conn.query_row(
            "WITH RECURSIVE counter(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM counter WHERE x<1000000000)
             SELECT SUM(x) FROM counter",[],|row|row.get::<_,i64>(0)
        ).map_err(|e|e.to_string())
    });
    assert!(result.is_err());
}

#[test]
fn locked_database_reports_error_and_is_not_treated_as_no_requests() {
    let path = std::env::temp_dir().join(format!("dialog-locked-{}.sqlite", uuid::Uuid::new_v4()));
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("CREATE TABLE lock_test(id INTEGER); BEGIN EXCLUSIVE")
        .unwrap();
    let reader =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    reader.busy_timeout(Duration::from_millis(10)).unwrap();
    assert!(read_data(
        &reader,
        &scope(),
        &query("chat", "logs"),
        Instant::now(),
        None
    )
    .is_err());
    writer.execute_batch("ROLLBACK").unwrap();
    drop(reader);
    drop(writer);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn summary_cache_only_matches_its_identity_and_expires() {
    let key = format!("dialog-cache-test-{}", uuid::Uuid::new_v4());
    remember_summary(key.clone(), json!({"requests":123}));
    assert_eq!(cached_summary(&key).unwrap()["requests"], 123);
    assert!(cached_summary(&format!("{key}-other")).is_none());
    SUMMARY_CACHE
        .lock()
        .unwrap()
        .iter_mut()
        .find(|entry| entry.key == key)
        .unwrap()
        .at = Instant::now() - Duration::from_secs(31);
    assert!(cached_summary(&key).is_none());
}

#[test]
fn local_service_binding_checks_configured_port_and_enabled_key() {
    let dir = std::env::temp_dir().join(format!("dialog-binding-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(dir.join("logs.sqlite")).unwrap();
    std::fs::write(dir.join("config.toml"),
        "model_provider='cockpit'\n[model_providers.cockpit]\nbase_url='http://localhost:58554/v1'\nexperimental_bearer_token='synthetic-binding-token'\n").unwrap();
    let mut config = json!({"port":58554,"apiKeys":[{"id":"key-a","key":"synthetic-binding-token","enabled":true}]});
    std::fs::write(dir.join("codex_local_access.json"), config.to_string()).unwrap();
    assert!(matches_local_service(&dir, &conn, &scope()).unwrap());
    let identity = cache_identity(&dir, &conn, &scope()).unwrap();
    config["apiKeys"][0]["tokenUsed"] = json!(99999);
    config["updatedAt"] = json!(99999);
    std::fs::write(dir.join("codex_local_access.json"), config.to_string()).unwrap();
    assert_eq!(identity, cache_identity(&dir, &conn, &scope()).unwrap());
    config["apiKeys"][0]["enabled"] = json!(false);
    std::fs::write(dir.join("codex_local_access.json"), config.to_string()).unwrap();
    assert!(!matches_local_service(&dir, &conn, &scope()).unwrap());
    assert_ne!(identity, cache_identity(&dir, &conn, &scope()).unwrap());
    config["apiKeys"][0]["enabled"] = json!(true);
    config["port"] = json!(58555);
    std::fs::write(dir.join("codex_local_access.json"), config.to_string()).unwrap();
    assert!(!matches_local_service(&dir, &conn, &scope()).unwrap());
    drop(conn);
    std::fs::remove_dir_all(dir).unwrap();
}

fn display_fixture_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dialog-names-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("codex_accounts")).unwrap();
    dir
}

#[test]
fn display_name_matches_api_service_name_and_preserves_actual_attempt_account() {
    let conn = database();
    append(&conn, "one", "key-a", THREAD, 1000);
    let mut actual = attempt("codex_apikey_actual", 200);
    actual["accountEmail"] = json!("api-key-1234abcd");
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1",
        [json!([actual]).to_string()],
    )
    .unwrap();
    let dir = display_fixture_dir();
    std::fs::write(
        dir.join("codex_accounts/codex_apikey_actual.json"),
        json!({
            "id":"codex_apikey_actual","email":"api-key-1234abcd","auth_mode":"apikey",
            "account_name":"DeepSeek","api_provider_mode":"custom",
            "api_base_url":"https://api.deepseek.com","openai_api_key":"SECRET_KEY",
            "tokens":{"access_token":"SECRET_TOKEN"}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("codex_accounts/account-a.json"),
        json!({
            "id":"account-a","email":"wrong@example.test","account_name":"Wrong account"
        })
        .to_string(),
    )
    .unwrap();
    let mut result = data(&conn, &scope(), &query("chat", "logs"));
    enrich_account_display_names(&dir, &mut result);
    assert_eq!(
        result["requests"]["rows"][0]["accountId"],
        "codex_apikey_actual"
    );
    assert_eq!(
        result["requests"]["rows"][0]["accountDisplayName"],
        "DeepSeek"
    );
    assert!(result["requests"]["rows"][0]["plan"].is_null());
    assert!(!result.to_string().contains("SECRET"));
    assert!(!result.to_string().contains("Wrong account"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_names_use_logged_email_but_never_generated_identifiers() {
    let dir = display_fixture_dir();
    let mut output = json!({"requests":{"rows":[
        {"accountId":"codex_old","accountEmail":"person@example.test"},
        {"accountId":"codex_apikey_old","accountEmail":"api-key-1234ABCD"},
        {"accountId":"codex_internal","accountEmail":"codex_internal"},
        {"accountId":"chatgpt_plan_missing","accountEmail":null}
    ]}});
    enrich_account_display_names(&dir, &mut output);
    assert_eq!(
        output["requests"]["rows"][0]["accountDisplayName"],
        "person@example.test"
    );
    for row in output["requests"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
    {
        assert!(row["accountDisplayName"].is_null());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn display_identity_uses_host_fallback_and_official_email_without_deserializing_credentials() {
    let api: DisplayIdentity = serde_json::from_value(json!({
        "id":"codex_apikey_example","email":"api-key-1234abcd","auth_mode":"apikey",
        "account_name":"api-key-1234abcd","api_provider_mode":"custom",
        "api_base_url":"https://private:SECRET@models.example.test:8443/v1",
        // These incompatible types prove the metadata DTO never deserializes
        // authentication members as credential structures.
        "openai_api_key":{"unexpected":"SECRET"},"tokens":["SECRET"]
    }))
    .unwrap();
    assert_eq!(
        display_identity_name(api).as_deref(),
        Some("models.example.test:8443")
    );
    let official: DisplayIdentity = serde_json::from_value(json!({
        "id":"chatgpt_plan_actual","email":"person@example.test","auth_mode":"apikey",
        "account_name":"ChatGPT","api_provider_id":"cockpit_chatgpt_plan",
        "api_base_url":"http://localhost:1234"
    }))
    .unwrap();
    assert_eq!(
        display_identity_name(official).as_deref(),
        Some("person@example.test")
    );
}

#[test]
fn display_metadata_read_is_bounded_and_rejects_wrong_identity_and_paths() {
    let dir = display_fixture_dir();
    std::fs::write(
        dir.join("codex_accounts/wrong.json"),
        json!({"id":"someone-else","email":"wrong@example.test"}).to_string(),
    )
    .unwrap();
    assert!(read_display_identity(&dir, "wrong").is_none());
    std::fs::write(
        dir.join("codex_accounts/large.json"),
        " ".repeat(1024 * 1024 + 1),
    )
    .unwrap();
    assert!(read_display_identity(&dir, "large").is_none());
    assert!(read_display_identity(&dir, "../outside").is_none());
    assert!(read_display_identity(&dir, "..").is_none());
    assert!(read_display_identity(&dir, "absent").is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_display_metadata_is_read_without_rotation_or_key_creation() {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
    use base64::{engine::general_purpose::STANDARD, Engine};
    let dir = display_fixture_dir();
    let plaintext = json!({
        "id":"codex_apikey_encrypted","email":"api-key-1234abcd","auth_mode":"apikey",
        "account_name":"DeepSeek","openai_api_key":"SECRET_KEY","tokens":{"access_token":"SECRET"}
    })
    .to_string();
    let key = [7u8; 32];
    let nonce = [5u8; 12];
    let ciphertext = Aes256Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .unwrap();
    let stored = json!({
        "version":1,"kind":"codex","algorithm":"AES-256-GCM","key_id":"fixture",
        "nonce":STANDARD.encode(nonce),"ciphertext":STANDARD.encode(ciphertext),
        "encrypted_at":0
    })
    .to_string();
    let path = dir.join("codex_accounts/codex_apikey_encrypted.json");
    std::fs::write(&path, &stored).unwrap();
    // A missing local key is unavailable; this observational path creates none.
    assert!(read_display_identity(&dir, "codex_apikey_encrypted").is_none());
    assert!(!dir.join("secure-account-storage.key").exists());
    std::fs::write(dir.join("secure-account-storage.key"), STANDARD.encode(key)).unwrap();
    assert_eq!(
        read_display_identity(&dir, "codex_apikey_encrypted").as_deref(),
        Some("DeepSeek")
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), stored);
    assert!(!dir.join("secure-account-storage.key.lock").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

fn gpt_count_fixture_config() -> Value {
    json!({
        "enabled":true,"port":58554,"apiKey":"fixture",
        "apiKeys":[{"id":"key-a","key":"fixture","label":"test","enabled":true,
            "inheritAccountPool":false,"accountIds":[]}],
        "accountIds":[],"createdAt":1,"updatedAt":1
    })
}

#[test]
fn gpt_count_is_overview_only_and_unavailable_metadata_does_not_erase_statistics() {
    let dir = display_fixture_dir();
    let identity = uuid::Uuid::new_v4().to_string();
    let mut overview = response(&query("chat", "overview"), true);
    overview["summary"] = json!({"requests":123});
    enrich_gpt_api_key_count(
        &dir,
        &scope(),
        &query("chat", "overview"),
        &identity,
        &mut overview,
    );
    assert!(overview["gptApiKeyCount"].is_null());
    assert_eq!(overview["summary"]["requests"], 123);
    std::fs::write(
        dir.join("codex_local_access.json"),
        gpt_count_fixture_config().to_string(),
    )
    .unwrap();
    enrich_gpt_api_key_count(
        &dir,
        &scope(),
        &query("chat", "overview"),
        &identity,
        &mut overview,
    );
    assert_eq!(overview["gptApiKeyCount"], 0);
    let mut logs = response(&query("chat", "logs"), true);
    enrich_gpt_api_key_count(&dir, &scope(), &query("chat", "logs"), &identity, &mut logs);
    assert!(logs.get("gptApiKeyCount").is_none());
    let mut unavailable = response(&query("chat", "overview"), false);
    enrich_gpt_api_key_count(
        &dir,
        &scope(),
        &query("chat", "overview"),
        &identity,
        &mut unavailable,
    );
    assert!(unavailable.get("gptApiKeyCount").is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn gpt_count_cache_is_key_scoped_and_missing_or_invalid_configuration_stays_unknown() {
    let dir = display_fixture_dir();
    let identity = uuid::Uuid::new_v4().to_string();
    let config = dir.join("codex_local_access.json");
    std::fs::write(&config, "{").unwrap();
    assert!(read_gpt_api_key_count(&dir, "key-a", &identity).is_none());
    std::fs::write(&config, gpt_count_fixture_config().to_string()).unwrap();
    assert_eq!(read_gpt_api_key_count(&dir, "key-a", &identity), Some(0));
    assert!(read_gpt_api_key_count(&dir, "unknown", &identity).is_none());
    std::fs::remove_file(&config).unwrap();
    assert_eq!(read_gpt_api_key_count(&dir, "key-a", &identity), Some(0));
    // A changed validated configuration identity cannot borrow the old count.
    assert!(read_gpt_api_key_count(&dir, "key-a", "changed-configuration").is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recorded_request_and_upstream_model_pair_preserves_aliases_and_unknown_history() {
    let conn = database();
    append(&conn, "alias", "key-a", THREAD, 1000);
    let old = data(&conn, &scope(), &query("chat", "logs"));
    assert!(old["requests"]["rows"][0]["requestedModel"].is_null());
    assert!(old["requests"]["rows"][0]["upstreamModel"].is_null());
    conn.execute_batch(
        "ALTER TABLE request_logs ADD COLUMN requested_model TEXT;
        ALTER TABLE request_logs ADD COLUMN upstream_model TEXT;
        UPDATE request_logs SET requested_model='gpt-client-shell',upstream_model='deepseek-v4';",
    )
    .unwrap();
    let result = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(result["requests"]["rows"][0]["model"], "model-a");
    assert_eq!(
        result["requests"]["rows"][0]["requestedModel"],
        "gpt-client-shell"
    );
    assert_eq!(
        result["requests"]["rows"][0]["upstreamModel"],
        "deepseek-v4"
    );
    let overview = data(&conn, &scope(), &query("chat", "overview"));
    assert_eq!(overview["session"]["requestedModel"], "gpt-client-shell");
    assert_eq!(overview["session"]["upstreamModel"], "deepseek-v4");
    conn.execute(
        "UPDATE request_logs SET upstream_model='gpt-client-shell'",
        [],
    )
    .unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"][0]["upstreamModel"],
        "gpt-client-shell"
    );
}

#[test]
fn authorization_route_never_comes_from_http_or_sse_protocol_names() {
    let conn = database();
    append(&conn, "protocol", "key-a", THREAD, 1000);
    let result = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(result["requests"]["rows"][0]["transport"], "http");
    assert_eq!(
        result["requests"]["rows"][0]["authorizationRoute"],
        "official"
    );
    let mut item = attempt("account-a", 200);
    item["authorizationRoute"] = json!("not-a-route");
    item["transport"] = json!("sse");
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1",
        [json!([item]).to_string()],
    )
    .unwrap();
    conn.execute("UPDATE request_logs SET authorization_route='http'", [])
        .unwrap();
    let unknown = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(unknown["requests"]["rows"][0]["transport"], "sse");
    assert!(unknown["requests"]["rows"][0]["authorizationRoute"].is_null());
    conn.execute(
        "UPDATE request_logs SET authorization_route='local_gateway'",
        [],
    )
    .unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"][0]["authorizationRoute"],
        "local_gateway"
    );
}

#[test]
fn recovered_requires_a_recorded_valid_official_same_account_fallback_trace() {
    let conn = database();
    append(&conn, "fallback", "key-a", THREAD, 1000);
    let mut local = attempt("account-a", 200);
    local["sequence"] = json!(2);
    local["authorizationRoute"] = json!("local_gateway");
    let trace = json!({"fromRoute":"official","toRoute":"local_gateway",
        "fromAccountId":"account-a","toAccountId":"account-a","reason":"official_throttled"});
    let mut variants = vec![(Value::Null, false), (trace.clone(), true)];
    for (field, value) in [
        ("toAccountId", "another"),
        ("fromRoute", "local_gateway"),
        ("toRoute", "official"),
        ("reason", ""),
        ("fromAccountId", ""),
    ] {
        let mut invalid = trace.clone();
        invalid[field] = json!(value);
        variants.push((invalid, false));
    }
    for (trace, expected) in variants {
        local["authorizationFallback"] = trace;
        conn.execute(
            "UPDATE request_diagnostics SET attempts_json=?1",
            [json!([attempt("account-a", 429), local.clone()]).to_string()],
        )
        .unwrap();
        assert_eq!(
            data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"][0]["recovered"],
            expected
        );
    }
}

#[test]
fn explicit_oauth_name_and_legacy_api_display_identity_match_api_page_semantics() {
    let oauth: DisplayIdentity = serde_json::from_value(json!({
        "id":"codex_oauth","email":"person@example.test","auth_mode":"oauth",
        "account_name":"  Work account  "
    }))
    .unwrap();
    assert_eq!(
        display_identity_name(oauth).as_deref(),
        Some("Work account")
    );
    let legacy: DisplayIdentity = serde_json::from_value(json!({
        "id":"codex_apikey_legacy","email":"api-key-1234abcd",
        "api_base_url":"https://models.example.test/v1"
    }))
    .unwrap();
    assert_eq!(
        display_identity_name(legacy).as_deref(),
        Some("models.example.test")
    );
    let unknown: DisplayIdentity = serde_json::from_value(json!({
        "id":"codex_apikey_legacy","email":"api-key-1234abcd"
    }))
    .unwrap();
    assert!(display_identity_name(unknown).is_none());
}

#[test]
fn recorded_zero_latency_is_kept_while_missing_or_invalid_timing_stays_unknown() {
    let conn = database();
    append(&conn, "zero", "key-a", THREAD, 1000);
    conn.execute(
        "UPDATE request_logs SET first_response_ms=0,latency_ms=0,output_generation_ms=0",
        [],
    )
    .unwrap();
    let result = data(&conn, &scope(), &query("chat", "logs"));
    assert_eq!(result["requests"]["rows"][0]["firstResponseMs"], 0);
    assert_eq!(result["requests"]["rows"][0]["latencyMs"], 0);
    assert_eq!(
        data(&conn, &scope(), &query("chat", "overview"))["session"]["firstResponseMs"],
        0
    );
    assert!(result["requests"]["rows"][0]["outputGenerationMs"].is_null());
    conn.execute(
        "UPDATE request_logs SET first_response_ms=-1,latency_ms=-1",
        [],
    )
    .unwrap();
    let invalid = data(&conn, &scope(), &query("chat", "logs"));
    assert!(invalid["requests"]["rows"][0]["firstResponseMs"].is_null());
    assert!(invalid["requests"]["rows"][0]["latencyMs"].is_null());
}

#[test]
fn future_summaries_keep_indexed_chat_pages_without_diagnostic_reads_or_history_backfill() {
    let conn = database();
    conn.execute_batch("CREATE INDEX idx_codex_local_access_logs_request_id ON request_logs(request_id,timestamp DESC)").unwrap();
    append(&conn, "historical", "key-a", THREAD, 500);
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    assert_eq!(
        data(&conn, &scope(), &query("chat", "logs"))["requests"]["rows"],
        json!([])
    );
    for n in 0..15 {
        let request = format!("new-{n:02}");
        append(&conn, &request, "key-a", THREAD, 1000);
        crate::modules::codex_request_summary::refresh_request(&conn, &request).unwrap();
    }
    for (request, key, thread) in [
        ("wrong-key", "key-b", THREAD),
        ("wrong-chat", "key-a", OTHER_THREAD),
    ] {
        append(&conn, request, key, thread, 2000);
        crate::modules::codex_request_summary::refresh_request(&conn, request).unwrap();
    }
    // The complete new projection remains queryable after diagnostic retention
    // expires. The UI never opens large bodies/attempts to reconstruct it.
    conn.execute_batch("DROP TABLE request_diagnostics")
        .unwrap();
    let mut q = query("chat", "logs");
    let first = data(&conn, &scope(), &q);
    assert_eq!(first["requests"]["rows"].as_array().unwrap().len(), 10);
    assert_eq!(first["requests"]["rows"][0]["requestId"], "new-14");
    assert!(first["session"].is_null());
    q.cursor = first["requests"]["nextCursor"].as_str().map(str::to_string);
    assert!(q.cursor.is_some());
    let second = data(&conn, &scope(), &q);
    assert_eq!(second["requests"]["rows"].as_array().unwrap().len(), 5);
    assert_eq!(second["requests"]["rows"][0]["requestId"], "new-04");
    assert!(second["requests"]["nextCursor"].is_null());
    q.thread_id = Some(OTHER_THREAD.into());
    assert!(read_data(&conn, &scope(), &q, Instant::now(), None).is_err());
    let plan = conn.query_row("EXPLAIN QUERY PLAN SELECT log_id FROM request_display_summaries INDEXED BY idx_request_display_chat WHERE api_key_id='key-a' AND client_instance_id='instance-a' AND client_thread_id=?1 ORDER BY timestamp DESC,log_id DESC LIMIT 11",[THREAD],|row|row.get::<_,String>(3)).unwrap();
    assert!(
        plan.contains("SEARCH") && plan.contains("idx_request_display_chat"),
        "{plan}"
    );
    assert!(!serde_json::to_string(&first).unwrap().contains("SECRET"));
    let service = data(&conn, &scope(), &query("service", "logs"));
    assert_eq!(service["requests"]["rows"][0]["requestId"], "wrong-chat");
}

#[test]
fn binding_fences_are_validated_and_part_of_pagination_identity() {
    let mut q = query("chat", "logs");
    q.binding = Some("a".repeat(64));
    assert!(parse_query(&serde_json::to_value(&q).unwrap()).is_some());
    let before = cursor_binding(&scope(), &q);
    q.binding = Some("b".repeat(64));
    assert_ne!(before, cursor_binding(&scope(), &q));
    for bad in ["a".repeat(63), "x".repeat(64), "../auth.json".into()] {
        q.binding = Some(bad);
        assert!(parse_query(&serde_json::to_value(&q).unwrap()).is_none());
    }
}
