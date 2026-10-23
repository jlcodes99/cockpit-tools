use super::*;
use serde_json::{json, Value};

const THREAD: &str = "019e92aa-0000-7000-8000-000000000001";
const OTHER_THREAD: &str = "019e92aa-0000-7000-8000-000000000002";

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE request_logs (
        id INTEGER PRIMARY KEY, account_id TEXT NOT NULL, email TEXT NOT NULL,
        timestamp INTEGER NOT NULL, client_instance_id TEXT NOT NULL,
        request_id TEXT NOT NULL, api_key_id TEXT NOT NULL);
        CREATE INDEX idx_codex_local_access_logs_api_key ON request_logs(api_key_id,timestamp DESC);
        CREATE TABLE request_diagnostics (
        request_id TEXT PRIMARY KEY,client_identity_captured INTEGER,client_thread_id TEXT,
        client_identity_metadata_captured INTEGER,client_installation_id TEXT,
        attempts_json TEXT,payloads_json TEXT); ",
    )
    .unwrap();
    conn
}

fn attempt(id: &str, sequence: u32, status: u16) -> Value {
    json!({"accountId":id,"accountEmail":format!("{id}@example.test"),
        "sequence":sequence,"transport":"responses","startedAtMs":sequence as i64*1000,
        "status":status,"success":status<400})
}

fn append(
    conn: &Connection,
    request: &str,
    key: &str,
    thread: &str,
    account: &str,
    time: i64,
    attempts: Value,
) {
    conn.execute("INSERT INTO request_logs(account_id,email,timestamp,client_instance_id,request_id,api_key_id)
        VALUES(?1,?2,?3,'',?4,?5)", rusqlite::params![account,format!("{account}@example.test"),time,request,key]).unwrap();
    conn.execute("INSERT INTO request_diagnostics VALUES(?1,1,?2,0,NULL,?3,'SECRET PAYLOAD MUST NOT BE READ')",
        rusqlite::params![request,thread,attempts.to_string()]).unwrap();
}

fn scope(key: &str) -> RequestScope {
    RequestScope {
        key_id: key.into(),
        ..Default::default()
    }
}

#[test]
fn recent_account_matches_exact_key_and_client_thread() {
    let conn = database();
    append(
        &conn,
        "a",
        "key-a",
        THREAD,
        "actual-a",
        1000,
        json!([attempt("actual-a", 1, 200)]),
    );
    append(
        &conn,
        "b",
        "key-b",
        THREAD,
        "other-key",
        3000,
        json!([attempt("other-key", 1, 200)]),
    );
    append(
        &conn,
        "c",
        "key-a",
        OTHER_THREAD,
        "other-chat",
        4000,
        json!([attempt("other-chat", 1, 200)]),
    );
    let actual = read_actual_account(&conn, THREAD, &scope("key-a"))
        .unwrap()
        .unwrap();
    assert_eq!(actual.account_id, "actual-a");
    assert_eq!(actual.email.as_deref(), Some("actual-a@example.test"));
    assert!(read_actual_account(&conn, OTHER_THREAD, &scope("key-b"))
        .unwrap()
        .is_none());
}

#[test]
fn recent_account_equal_timestamps_use_last_inserted_actual_request() {
    let conn = database();
    append(
        &conn,
        "first",
        "key",
        THREAD,
        "first",
        1000,
        json!([attempt("first", 1, 200)]),
    );
    append(
        &conn,
        "second",
        "key",
        THREAD,
        "second",
        1000,
        json!([attempt("second", 1, 200)]),
    );
    assert_eq!(
        read_actual_account(&conn, THREAD, &scope("key"))
            .unwrap()
            .unwrap()
            .account_id,
        "second"
    );
}

#[test]
fn recent_account_uses_final_retry_and_accepts_real_failed_attempt() {
    let conn = database();
    append(
        &conn,
        "retry",
        "key",
        THREAD,
        "final",
        3000,
        json!([attempt("first", 1, 429), attempt("final", 2, 200)]),
    );
    let actual = read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .unwrap();
    assert_eq!(actual.account_id, "final");
    assert_eq!(actual.last_used_at_ms, 2000);
    append(
        &conn,
        "failed",
        "key",
        THREAD,
        "failed-final",
        4000,
        json!([attempt("first", 1, 429), attempt("failed-final", 2, 401)]),
    );
    assert_eq!(
        read_actual_account(&conn, THREAD, &scope("key"))
            .unwrap()
            .unwrap()
            .account_id,
        "failed-final"
    );
}

#[test]
fn recent_account_never_uses_preferred_or_preparation_only_account() {
    let conn = database();
    append(
        &conn,
        "old",
        "key",
        THREAD,
        "old",
        1000,
        json!([attempt("old", 1, 200)]),
    );
    let mut prepared = attempt("preferred", 1, 503);
    prepared["transport"] = json!("preflight");
    prepared["failurePhase"] = json!("routing");
    append(
        &conn,
        "prepared",
        "key",
        THREAD,
        "preferred",
        2000,
        json!([prepared]),
    );
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    let mut not_sent = attempt("selected", 1, 503);
    not_sent.as_object_mut().unwrap().remove("status");
    append(
        &conn,
        "selected",
        "key",
        THREAD,
        "selected",
        3000,
        json!([not_sent]),
    );
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    append(&conn, "routing-failed", "key", THREAD, "", 4000, json!([]));
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
}

#[test]
fn recent_account_does_not_guess_when_final_trace_is_missing_or_disagrees() {
    let conn = database();
    append(
        &conn,
        "mismatch",
        "key",
        THREAD,
        "final",
        1000,
        json!([attempt("earlier", 1, 200)]),
    );
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    conn.execute("UPDATE request_diagnostics SET attempts_json='[]'", [])
        .unwrap();
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    conn.execute(
        "UPDATE request_diagnostics SET attempts_json=?1",
        ["x".repeat(65537)],
    )
    .unwrap();
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
}

#[test]
fn recent_account_ignores_later_preflight_after_last_real_attempt() {
    let conn = database();
    let mut preflight = attempt("never-sent", 2, 503);
    preflight["transport"] = json!("preflight");
    append(
        &conn,
        "preflight",
        "key",
        THREAD,
        "actual",
        2000,
        json!([attempt("actual", 1, 429), preflight]),
    );
    assert_eq!(
        read_actual_account(&conn, THREAD, &scope("key"))
            .unwrap()
            .unwrap()
            .account_id,
        "actual"
    );
}

#[test]
fn recent_account_requires_captured_identity_and_respects_explicit_profile_headers() {
    let conn = database();
    append(
        &conn,
        "a",
        "key",
        THREAD,
        "actual",
        1000,
        json!([attempt("actual", 1, 200)]),
    );
    let mut expected = scope("key");
    expected.instance_id = Some("instance-a".into());
    assert!(read_actual_account(&conn, THREAD, &expected)
        .unwrap()
        .is_none());
    conn.execute(
        "UPDATE request_logs SET client_instance_id='instance-a'",
        [],
    )
    .unwrap();
    expected.installation_id = Some("installation-a".into());
    assert!(read_actual_account(&conn, THREAD, &expected)
        .unwrap()
        .is_none());
    conn.execute("UPDATE request_diagnostics SET client_identity_metadata_captured=1,client_installation_id='installation-a'",[]).unwrap();
    assert!(read_actual_account(&conn, THREAD, &expected)
        .unwrap()
        .is_some());
    conn.execute(
        "UPDATE request_diagnostics SET client_identity_captured=0",
        [],
    )
    .unwrap();
    assert!(read_actual_account(&conn, THREAD, &expected)
        .unwrap()
        .is_none());
}

#[test]
fn recent_account_bounds_history_and_never_reads_payloads() {
    let conn = database();
    append(
        &conn,
        "old",
        "key",
        THREAD,
        "old",
        1,
        json!([attempt("old", 1, 200)]),
    );
    for n in 0..RECENT_LOG_LIMIT {
        append(
            &conn,
            &format!("recent-{n}"),
            "key",
            OTHER_THREAD,
            "other",
            n + 2,
            json!([attempt("other", 1, 200)]),
        );
    }
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    append(
        &conn,
        "latest",
        "key",
        THREAD,
        "actual",
        1000,
        json!([attempt("actual", 1, 200)]),
    );
    // Corrupt/non-JSON content is harmless because payloads are never selected.
    conn.execute(
        "UPDATE request_diagnostics SET payloads_json=?1 WHERE request_id='latest'",
        ["SECRET".repeat(100000)],
    )
    .unwrap();
    assert_eq!(
        read_actual_account(&conn, THREAD, &scope("key"))
            .unwrap()
            .unwrap()
            .account_id,
        "actual"
    );
}

#[test]
fn recent_account_reads_old_schema_without_migrating_or_failing() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE request_logs(id INTEGER PRIMARY KEY)")
        .unwrap();
    assert!(read_actual_account(&conn, THREAD, &scope("key"))
        .unwrap()
        .is_none());
    let columns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('request_logs')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 1);
}

#[test]
fn recent_account_resolves_real_bearer_key_without_preferred_account_guessing() {
    let keys:StoredKeys=serde_json::from_value(json!({"gatewayMode":"native","apiKey":"legacy-secret",
        "preferredAccountId":"not-actual","apiKeys":[{"id":"real-key","key":"bearer-secret","preferredAccountId":"wrong"}]})).unwrap();
    assert_eq!(
        key_id_for_bearer(&keys, "bearer-secret").as_deref(),
        Some("real-key")
    );
    assert_eq!(key_id_for_bearer(&keys, "legacy-secret"), None);
    assert_eq!(key_id_for_bearer(&keys, "unknown"), None);
    let native: StoredKeys =
        serde_json::from_value(json!({"gatewayMode":"native","apiKey":"secret"})).unwrap();
    assert_eq!(
        key_id_for_bearer(&native, "secret").as_deref(),
        Some("native:primary")
    );
    let sidecar: StoredKeys =
        serde_json::from_value(json!({"gatewayMode":"sidecar","apiKey":"secret"})).unwrap();
    assert_eq!(
        key_id_for_bearer(&sidecar, "secret").as_deref(),
        Some("legacy")
    );
}

#[test]
fn recent_account_rejects_account_path_escape_and_limits_configuration_bytes() {
    for id in ["../secret", "a/b", "a\\b", "..", "", "a\n"] {
        assert!(!safe_account_id(id));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.json");
    std::fs::write(&path, vec![b'x'; CONFIG_LIMIT as usize + 1]).unwrap();
    assert!(read_keys(&path).is_err());
}

#[test]
fn recent_account_profile_scope_uses_only_explicit_wire_headers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        r#"
model_provider = "gateway"
[model_providers.gateway]
base_url = "http://127.0.0.1:1234/v1"
experimental_bearer_token = "fixture-key"
[model_providers.gateway.http_headers]
x-cockpit-instance-id = "fixture-instance"
x-codex-installation-id = "fixture-installation"
"#,
    )
    .unwrap();
    let (url, key, scope) = profile_gateway(dir.path()).unwrap().unwrap();
    assert_eq!(url, "http://127.0.0.1:1234/v1");
    assert_eq!(key, "fixture-key");
    assert_eq!(scope.instance_id.as_deref(), Some("fixture-instance"));
    assert_eq!(
        scope.installation_id.as_deref(),
        Some("fixture-installation")
    );
}

#[test]
fn recent_account_busy_database_returns_error_without_waiting_for_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("logs.sqlite");
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("CREATE TABLE request_logs(id INTEGER PRIMARY KEY);BEGIN EXCLUSIVE;")
        .unwrap();
    let reader = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    reader.busy_timeout(Duration::from_millis(20)).unwrap();
    let started = Instant::now();
    assert!(read_actual_account(&reader, THREAD, &scope("key")).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}
