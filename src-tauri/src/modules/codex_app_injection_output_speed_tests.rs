use super::*;

const THREAD: &str = "019e92aa-0000-7000-8000-000000000001";
const CHILD_THREAD: &str = "019e92aa-0000-7000-8000-000000000002";

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE request_logs (
            id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL,
            client_instance_id TEXT NOT NULL, success INTEGER NOT NULL,
            error_category TEXT NOT NULL, output_tokens INTEGER NOT NULL,
            output_generation_ms INTEGER, request_id TEXT NOT NULL,
            api_key_id TEXT NOT NULL, model_id TEXT, requested_model TEXT,
            requested_service_tier TEXT, response_service_tier TEXT,
            input_tokens INTEGER, cached_tokens INTEGER);
        CREATE INDEX idx_codex_local_access_logs_api_key
            ON request_logs(api_key_id,timestamp DESC);
        CREATE TABLE request_diagnostics (
            request_id TEXT PRIMARY KEY, client_identity_captured INTEGER,
            client_thread_id TEXT, client_identity_metadata_captured INTEGER,
            client_installation_id TEXT);",
    )
    .unwrap();
}

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    conn
}

struct Sample<'a> {
    key: &'a str,
    instance: &'a str,
    thread: Option<&'a str>,
    installation: Option<&'a str>,
    captured: Option<i64>,
    metadata_captured: Option<i64>,
    timestamp: i64,
    success: bool,
    error: &'a str,
    tokens: i64,
    generation_ms: Option<i64>,
    model: Option<&'a str>,
    requested_model: Option<&'a str>,
    requested_tier: Option<&'a str>,
    response_tier: Option<&'a str>,
    input_tokens: Option<i64>,
    cached_tokens: Option<i64>,
}

impl Default for Sample<'_> {
    fn default() -> Self {
        Self {
            key: "key-a",
            instance: "instance-a",
            thread: Some(THREAD),
            installation: None,
            captured: Some(1),
            metadata_captured: Some(1),
            timestamp: 1000,
            success: true,
            error: "",
            tokens: 200,
            generation_ms: Some(2000),
            model: Some("gpt-6.1-sol"),
            requested_model: None,
            requested_tier: None,
            response_tier: None,
            input_tokens: None,
            cached_tokens: None,
        }
    }
}

fn append(conn: &Connection, request: &str, sample: Sample<'_>) {
    conn.execute(
        "INSERT INTO request_logs(timestamp,client_instance_id,success,error_category,
            output_tokens,output_generation_ms,request_id,api_key_id,model_id,requested_model,
            requested_service_tier,response_service_tier,input_tokens,cached_tokens)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        rusqlite::params![
            sample.timestamp,
            sample.instance,
            sample.success,
            sample.error,
            sample.tokens,
            sample.generation_ms,
            request,
            sample.key,
            sample.model,
            sample.requested_model,
            sample.requested_tier,
            sample.response_tier,
            sample.input_tokens,
            sample.cached_tokens,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO request_diagnostics VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            request,
            sample.captured,
            sample.thread,
            sample.metadata_captured,
            sample.installation
        ],
    )
    .unwrap();
}

fn scope() -> RequestScope {
    RequestScope {
        key_id: "key-a".into(),
        instance_id: Some("instance-a".into()),
        installation_id: None,
    }
}

fn read(conn: &Connection, scope: &RequestScope) -> Option<ThreadOutputSpeed> {
    read_samples(conn, THREAD, scope, Instant::now()).unwrap()
}

#[test]
fn output_speed_accepts_real_client_without_optional_installation_header() {
    let conn = database();
    append(&conn, "real-client", Sample::default());
    let speed = read(&conn, &scope()).unwrap();
    assert_eq!(speed.thread_id, THREAD);
    assert_eq!(speed.tokens_per_second, 100.0);
    assert_eq!(speed.samples, 1);
    assert_eq!(speed.observed_at_ms, 1000);
}

#[test]
fn output_speed_requires_explicit_installation_to_match_when_configured() {
    let conn = database();
    let mut expected = scope();
    expected.installation_id = Some("installation-a".into());
    append(&conn, "missing", Sample::default());
    append(
        &conn,
        "other",
        Sample {
            installation: Some("installation-b"),
            ..Default::default()
        },
    );
    assert!(read(&conn, &expected).is_none());
    append(
        &conn,
        "exact",
        Sample {
            installation: Some("installation-a"),
            ..Default::default()
        },
    );
    let speed = read(&conn, &expected).unwrap();
    assert_eq!(speed.samples, 1);
    assert_eq!(speed.tokens_per_second, 100.0);
}

#[test]
fn output_speed_isolates_key_instance_parent_and_child_threads() {
    let conn = database();
    for (request, sample) in [
        (
            "other-key",
            Sample {
                key: "key-b",
                ..Default::default()
            },
        ),
        (
            "other-instance",
            Sample {
                instance: "instance-b",
                ..Default::default()
            },
        ),
        (
            "child",
            Sample {
                thread: Some(CHILD_THREAD),
                ..Default::default()
            },
        ),
    ] {
        append(&conn, request, sample);
    }
    assert!(read(&conn, &scope()).is_none());
    append(
        &conn,
        "parent",
        Sample {
            tokens: 300,
            ..Default::default()
        },
    );
    let parent = read(&conn, &scope()).unwrap();
    assert_eq!(parent.samples, 1);
    assert_eq!(parent.tokens_per_second, 150.0);
    let child = read_samples(&conn, CHILD_THREAD, &scope(), Instant::now())
        .unwrap()
        .unwrap();
    assert_eq!(child.samples, 1);
    assert_eq!(child.tokens_per_second, 100.0);
}

#[test]
fn output_speed_never_guesses_missing_or_uncaptured_identity() {
    for sample in [
        Sample {
            captured: None,
            ..Default::default()
        },
        Sample {
            captured: Some(0),
            ..Default::default()
        },
        Sample {
            metadata_captured: None,
            ..Default::default()
        },
        Sample {
            metadata_captured: Some(0),
            ..Default::default()
        },
        Sample {
            thread: None,
            ..Default::default()
        },
        Sample {
            thread: Some("invalid-thread"),
            ..Default::default()
        },
    ] {
        let conn = database();
        append(&conn, "invalid", sample);
        assert!(read(&conn, &scope()).is_none());
    }
    let conn = database();
    append(&conn, "missing-diagnostics", Sample::default());
    conn.execute("DELETE FROM request_diagnostics", []).unwrap();
    assert!(read(&conn, &scope()).is_none());
}

#[test]
fn output_speed_uses_generation_timing_and_rejects_ineligible_samples() {
    for sample in [
        Sample {
            generation_ms: None,
            ..Default::default()
        },
        Sample {
            generation_ms: Some(99),
            ..Default::default()
        },
        Sample {
            generation_ms: Some(0),
            ..Default::default()
        },
        Sample {
            generation_ms: Some(-1),
            ..Default::default()
        },
        Sample {
            generation_ms: Some(MAX_SAFE_INTEGER + 1),
            ..Default::default()
        },
        Sample {
            tokens: 99,
            ..Default::default()
        },
        Sample {
            tokens: -1,
            ..Default::default()
        },
        Sample {
            tokens: MAX_SAFE_INTEGER + 1,
            ..Default::default()
        },
        Sample {
            success: false,
            ..Default::default()
        },
        Sample {
            error: "upstream_failure",
            ..Default::default()
        },
    ] {
        let conn = database();
        append(&conn, "ineligible", sample);
        assert!(read(&conn, &scope()).is_none());
    }
    let conn = database();
    append(
        &conn,
        "threshold",
        Sample {
            tokens: 100,
            generation_ms: Some(100),
            ..Default::default()
        },
    );
    assert_eq!(read(&conn, &scope()).unwrap().tokens_per_second, 1000.0);
}

#[test]
fn output_speed_weights_tokens_and_generation_time_instead_of_averaging_rates() {
    let conn = database();
    append(
        &conn,
        "fast",
        Sample {
            tokens: 1000,
            generation_ms: Some(1000),
            timestamp: 100,
            ..Default::default()
        },
    );
    append(
        &conn,
        "slow",
        Sample {
            tokens: 100,
            generation_ms: Some(9000),
            timestamp: 200,
            ..Default::default()
        },
    );
    append(
        &conn,
        "failed-newest",
        Sample {
            success: false,
            timestamp: 300,
            ..Default::default()
        },
    );
    let speed = read(&conn, &scope()).unwrap();
    assert_eq!(speed.tokens_per_second, 110.0);
    assert_eq!(speed.samples, 2);
    assert_eq!(speed.observed_at_ms, 200);
}

#[test]
fn output_speed_rejects_totals_outside_javascript_safe_range() {
    for (tokens, generation_ms) in [(MAX_SAFE_INTEGER, 100), (100, MAX_SAFE_INTEGER)] {
        let conn = database();
        for request in ["one", "two"] {
            append(
                &conn,
                request,
                Sample {
                    tokens,
                    generation_ms: Some(generation_ms),
                    ..Default::default()
                },
            );
        }
        assert!(read(&conn, &scope()).is_none());
    }
}

#[test]
fn output_speed_only_reads_latest_256_requests_for_the_selected_key() {
    let conn = database();
    append(
        &conn,
        "too-old",
        Sample {
            timestamp: 1,
            ..Default::default()
        },
    );
    for n in 0..RECENT_LOG_LIMIT {
        append(
            &conn,
            &format!("child-{n}"),
            Sample {
                thread: Some(CHILD_THREAD),
                timestamp: n + 2,
                ..Default::default()
            },
        );
    }
    assert!(read(&conn, &scope()).is_none());
    append(
        &conn,
        "current",
        Sample {
            timestamp: 1000,
            tokens: 400,
            ..Default::default()
        },
    );
    for n in 0..RECENT_LOG_LIMIT {
        append(
            &conn,
            &format!("other-key-{n}"),
            Sample {
                key: "key-b",
                timestamp: 2000 + n,
                ..Default::default()
            },
        );
    }
    let speed = read(&conn, &scope()).unwrap();
    assert_eq!(speed.samples, 1);
    assert_eq!(speed.tokens_per_second, 200.0);
}

#[test]
fn output_speed_old_schema_returns_none_without_migration() {
    for setup in ["CREATE TABLE request_logs(id INTEGER PRIMARY KEY)", ""] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(setup).unwrap();
        let before: i64 = conn
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        assert!(read(&conn, &scope()).is_none());
        let after: i64 = conn
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after);
    }
    for column in ["output_generation_ms", "client_identity_metadata_captured"] {
        let conn = database();
        let table = if column == "output_generation_ms" {
            "request_logs"
        } else {
            "request_diagnostics"
        };
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
            .unwrap();
        assert!(read(&conn, &scope()).is_none());
    }
}

#[test]
fn output_speed_query_budget_does_not_return_stale_partial_result() {
    let conn = database();
    append(&conn, "sample", Sample::default());
    assert!(read_samples(
        &conn,
        THREAD,
        &scope(),
        Instant::now() - Duration::from_secs(1)
    )
    .is_err());
}

fn selected(model: &str, tier: &str) -> OutputSpeedSelection {
    OutputSpeedSelection {
        model: Some(model.into()),
        service_tier: Some(tier.into()),
    }
}

fn read_selected(conn: &Connection, selection: &OutputSpeedSelection) -> Option<ThreadOutputSpeed> {
    read_samples_with_selection(conn, THREAD, &scope(), selection, Instant::now()).unwrap()
}

#[test]
fn output_speed_latest_fast_measurement_is_not_the_session_average() {
    let conn = database();
    append(
        &conn,
        "old-normal",
        Sample {
            tokens: 10_000,
            generation_ms: Some(400_000),
            timestamp: 100,
            requested_tier: Some("default"),
            ..Default::default()
        },
    );
    append(
        &conn,
        "new-fast",
        Sample {
            tokens: 214,
            generation_ms: Some(3242),
            timestamp: 200,
            requested_tier: Some("priority"),
            response_tier: Some("default"),
            input_tokens: Some(112_000),
            cached_tokens: Some(107_000),
            ..Default::default()
        },
    );
    let fast = read_selected(&conn, &selected("gpt-6.1-sol", "fast")).unwrap();
    assert!(fast.tokens_per_second > 25.0 && fast.tokens_per_second < 26.0);
    assert_eq!(fast.latest_tokens_per_second.unwrap().round(), 66.0);
    assert_eq!(fast.request_id.as_deref(), Some("new-fast"));
    assert_eq!(fast.latest_observed_at_ms, Some(200));
    assert_eq!(fast.selection.service_tier.as_deref(), Some("priority"));
    assert_eq!(fast.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(fast.response_service_tier.as_deref(), Some("default"));
    assert_eq!(fast.latest_cache_hit_rate, Some(107_000.0 / 112_000.0));
    // Reading again after generation has stopped returns the same completed request.
    let stopped = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(stopped.request_id, fast.request_id);
    assert_eq!(
        stopped.latest_tokens_per_second,
        fast.latest_tokens_per_second
    );
    assert_eq!(stopped.latest_cache_hit_rate, fast.latest_cache_hit_rate);
    let normal = read_selected(&conn, &selected("gpt-6.1-sol", "standard")).unwrap();
    assert_eq!(normal.latest_tokens_per_second, Some(25.0));
    assert_eq!(normal.request_id.as_deref(), Some("old-normal"));
}

#[test]
fn output_speed_latest_isolates_model_tier_and_rejects_missing_evidence() {
    let conn = database();
    append(
        &conn,
        "normal",
        Sample {
            requested_tier: Some("default"),
            timestamp: 100,
            ..Default::default()
        },
    );
    append(
        &conn,
        "fast",
        Sample {
            requested_tier: Some("priority"),
            timestamp: 200,
            ..Default::default()
        },
    );
    append(
        &conn,
        "other-model",
        Sample {
            model: Some("deepseek-flash"),
            requested_tier: Some("priority"),
            timestamp: 300,
            ..Default::default()
        },
    );
    append(
        &conn,
        "tier-unknown",
        Sample {
            requested_tier: None,
            timestamp: 400,
            ..Default::default()
        },
    );
    append(
        &conn,
        "model-unknown",
        Sample {
            model: None,
            requested_tier: Some("priority"),
            timestamp: 500,
            ..Default::default()
        },
    );
    let fast = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(fast.request_id.as_deref(), Some("fast"));
    assert_eq!(fast.samples, 5);
    assert!(read_selected(&conn, &selected("gpt-6.1-sol", "auto"))
        .unwrap()
        .latest_tokens_per_second
        .is_none());
    assert_eq!(
        read_selected(
            &conn,
            &OutputSpeedSelection {
                model: Some("gpt-6.1-sol".into()),
                service_tier: None
            }
        )
        .unwrap()
        .request_id
        .as_deref(),
        Some("tier-unknown")
    );
    assert_eq!(
        read_selected(&conn, &selected("deepseek-flash", "fast"))
            .unwrap()
            .request_id
            .as_deref(),
        Some("other-model")
    );
}

#[test]
fn output_speed_requested_model_and_timestamp_ties_are_deterministic() {
    let conn = database();
    append(
        &conn,
        "first",
        Sample {
            requested_tier: Some("priority"),
            ..Default::default()
        },
    );
    append(
        &conn,
        "second",
        Sample {
            model: Some("actual-upstream"),
            requested_model: Some("gpt-6.1-sol"),
            requested_tier: Some("priority"),
            tokens: 400,
            ..Default::default()
        },
    );
    let speed = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(speed.request_id.as_deref(), Some("second"));
    assert_eq!(speed.latest_tokens_per_second, Some(200.0));
    assert_eq!(speed.model.as_deref(), Some("gpt-6.1-sol"));
}

#[test]
fn output_speed_latest_cache_can_be_reliable_when_speed_is_not() {
    let conn = database();
    append(
        &conn,
        "older-speed",
        Sample {
            timestamp: 100,
            requested_tier: Some("priority"),
            input_tokens: Some(1000),
            cached_tokens: Some(900),
            ..Default::default()
        },
    );
    append(
        &conn,
        "new-cache-only",
        Sample {
            timestamp: 200,
            tokens: 20,
            generation_ms: None,
            requested_tier: Some("priority"),
            input_tokens: Some(100),
            cached_tokens: Some(0),
            ..Default::default()
        },
    );
    let speed = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(speed.request_id.as_deref(), Some("new-cache-only"));
    assert_eq!(speed.latest_tokens_per_second, None);
    assert_eq!(speed.latest_input_tokens, Some(100));
    assert_eq!(speed.latest_cached_tokens, Some(0));
    assert_eq!(speed.latest_cache_hit_rate, Some(0.0));
    assert_eq!(speed.tokens_per_second, 100.0);
    assert_eq!(speed.avg_cache_hit_rate, Some(900.0 / 1100.0));
    assert_eq!(speed.total_input_tokens, Some(1100));
    assert_eq!(speed.total_cached_tokens, Some(900));
    let payload = serde_json::to_value(&speed).unwrap();
    assert_eq!(payload["totalInputTokens"], 1100);
    assert_eq!(payload["totalCachedTokens"], 900);
    conn.execute(
        "DELETE FROM request_logs WHERE request_id='older-speed'",
        [],
    )
    .unwrap();
    let cache_only = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(cache_only.samples, 0);
    assert_eq!(cache_only.tokens_per_second, 0.0);
    assert_eq!(cache_only.total_input_tokens, Some(100));
    assert_eq!(cache_only.total_cached_tokens, Some(0));
    assert!(serde_json::to_string(&cache_only)
        .unwrap()
        .contains("\"latestCacheHitRate\":0.0"));
}

#[test]
fn output_speed_latest_does_not_borrow_cache_from_a_different_request() {
    let conn = database();
    append(
        &conn,
        "old-cache",
        Sample {
            timestamp: 100,
            requested_tier: Some("priority"),
            input_tokens: Some(1000),
            cached_tokens: Some(950),
            ..Default::default()
        },
    );
    append(
        &conn,
        "new-speed-only",
        Sample {
            timestamp: 200,
            requested_tier: Some("priority"),
            ..Default::default()
        },
    );
    append(
        &conn,
        "newest-no-metrics",
        Sample {
            timestamp: 300,
            requested_tier: Some("priority"),
            tokens: 0,
            generation_ms: None,
            input_tokens: Some(0),
            cached_tokens: Some(0),
            ..Default::default()
        },
    );
    let speed = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(speed.request_id.as_deref(), Some("new-speed-only"));
    assert_eq!(speed.latest_tokens_per_second, Some(100.0));
    assert_eq!(speed.latest_input_tokens, None);
    assert_eq!(speed.latest_cached_tokens, None);
    assert_eq!(speed.latest_cache_hit_rate, None);
}

#[test]
fn output_speed_cache_rejects_inconsistent_or_unsafe_counters() {
    for (input, cached) in [
        (0, 0),
        (-1, 0),
        (100, -1),
        (100, 101),
        (MAX_SAFE_INTEGER + 1, 50),
    ] {
        let conn = database();
        append(
            &conn,
            "bad-cache",
            Sample {
                requested_tier: Some("priority"),
                input_tokens: Some(input),
                cached_tokens: Some(cached),
                ..Default::default()
            },
        );
        let speed = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
        assert_eq!(speed.latest_cache_hit_rate, None);
        assert_eq!(speed.avg_cache_hit_rate, None);
    }
}

#[test]
fn output_speed_unspecified_tier_only_matches_an_unspecified_request() {
    let conn = database();
    append(
        &conn,
        "unspecified",
        Sample {
            timestamp: 100,
            input_tokens: Some(100),
            cached_tokens: Some(0),
            ..Default::default()
        },
    );
    append(
        &conn,
        "invalid-not-unspecified",
        Sample {
            timestamp: 200,
            requested_tier: Some("unrecognized-mode"),
            ..Default::default()
        },
    );
    append(
        &conn,
        "priority",
        Sample {
            timestamp: 300,
            requested_tier: Some("priority"),
            ..Default::default()
        },
    );
    let unspecified = OutputSpeedSelection {
        model: Some("gpt-6.1-sol".into()),
        service_tier: None,
    };
    let measured = read_selected(&conn, &unspecified).unwrap();
    assert_eq!(measured.request_id.as_deref(), Some("unspecified"));
    assert_eq!(measured.requested_service_tier, None);
    assert_eq!(measured.latest_cache_hit_rate, Some(0.0));
    assert!(read_selected(&conn, &selected("gpt-6.1-sol", "default"))
        .unwrap()
        .request_id
        .is_none());
}

#[test]
fn output_speed_standard_mode_accepts_omitted_request_tier_only_with_response_evidence() {
    for use_summary in [false, true] {
        let conn = database();
        if use_summary {
            create_summary_schema(&conn);
        }
        let cases = [
            (None, Some("default"), true),
            (Some(""), Some("default"), true),
            (Some("  "), Some("default"), true),
            (None, None, false),
            (None, Some("priority"), false),
            (Some("priority"), Some("default"), false),
            (Some("auto"), Some("default"), false),
            (Some("unrecognized-mode"), Some("default"), false),
        ];
        for (index, (requested, returned, matches_standard)) in cases.into_iter().enumerate() {
            let id = index as i64 + 1;
            let request_id = format!("request-{id}");
            append(
                &conn,
                &request_id,
                Sample {
                    timestamp: id,
                    requested_tier: requested,
                    response_tier: returned,
                    tokens: 234,
                    generation_ms: Some(5842),
                    input_tokens: Some(58827),
                    cached_tokens: Some(48512),
                    ..Default::default()
                },
            );
            if use_summary {
                append_summary(&conn, id, "key-a", THREAD, true);
                let value = serde_json::json!({
                    "model":"gpt-6.1-sol", "requestedModel":"gpt-6.1-sol",
                    "requestedServiceTier":requested, "responseServiceTier":returned,
                    "success":true, "outputTokens":234, "outputGenerationMs":5842,
                    "inputTokens":58827, "cachedTokens":48512
                });
                conn.execute(
                    "UPDATE request_display_summaries SET data_json=?1 WHERE log_id=?2",
                    rusqlite::params![value.to_string(), id],
                )
                .unwrap();
            }
            let measured = read_selected(&conn, &selected("gpt-6.1-sol", "default")).unwrap();
            assert_eq!(
                measured.request_id.as_deref(),
                Some(if matches_standard {
                    request_id.as_str()
                } else {
                    "request-3"
                }),
                "summary={use_summary}, requested={requested:?}, returned={returned:?}"
            );
            assert_eq!(measured.latest_tokens_per_second, Some(234000.0 / 5842.0));
            assert_eq!(measured.latest_cache_hit_rate, Some(48512.0 / 58827.0));
            if requested.is_none() || requested.is_some_and(|value| value.trim().is_empty()) {
                for tier in ["priority", "ultrafast", "auto", "flex"] {
                    assert!(read_selected(&conn, &selected("gpt-6.1-sol", tier))
                        .unwrap()
                        .request_id
                        .is_none());
                }
            }
        }
    }
}

fn create_summary_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE request_display_summaries (
        log_id INTEGER PRIMARY KEY,request_id TEXT NOT NULL,timestamp INTEGER,
        api_key_id TEXT,client_instance_id TEXT,client_thread_id TEXT,
        client_installation_id TEXT,client_identity_captured INTEGER,
        client_identity_metadata_captured INTEGER,dispatched INTEGER,data_json TEXT);
        CREATE INDEX idx_request_display_chat ON request_display_summaries
        (api_key_id,client_instance_id,client_thread_id,timestamp DESC,log_id DESC);",
    )
    .unwrap();
}

fn append_summary(conn: &Connection, id: i64, key: &str, thread: &str, captured: bool) {
    let value = serde_json::json!({"model":"gpt-6.1-sol","requestedServiceTier":"priority",
        "success":true,"outputTokens":200,"outputGenerationMs":2000,"inputTokens":1000,"cachedTokens":750});
    conn.execute(
        "INSERT INTO request_display_summaries VALUES(?1,?2,?1,?3,'instance-a',?4,NULL,?5,?5,1,?6)",
        rusqlite::params![
            id,
            format!("request-{id}"),
            key,
            thread,
            captured,
            value.to_string()
        ],
    )
    .unwrap();
}

#[test]
fn output_speed_new_summaries_use_exact_thread_index_without_history_backfill() {
    let conn = database();
    append(&conn, "old-history", Sample::default());
    create_summary_schema(&conn);
    let before: i64 = conn
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .unwrap();
    assert!(read(&conn, &scope()).is_none());
    append_summary(&conn, 1, "key-a", THREAD, true);
    for id in 2..800 {
        append_summary(&conn, id, "key-a", CHILD_THREAD, true)
    }
    append_summary(&conn, 1000, "key-b", THREAD, true);
    append_summary(&conn, 1001, "key-a", THREAD, false);
    let speed = read_selected(&conn, &selected("gpt-6.1-sol", "priority")).unwrap();
    assert_eq!(speed.samples, 1);
    assert_eq!(speed.request_id.as_deref(), Some("request-1"));
    assert_eq!(speed.latest_tokens_per_second, Some(100.0));
    assert_eq!(speed.latest_cache_hit_rate, Some(0.75));
    let after: i64 = conn
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
    let plan: String = conn
        .query_row(
            &format!("EXPLAIN QUERY PLAN {}", summary_query()),
            rusqlite::params!["key-a", "instance-a", THREAD, RECENT_LOG_LIMIT],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("idx_request_display_chat"));
}

#[test]
fn output_speed_rejects_oversized_summary_json_before_parsing_measurements() {
    let conn = database();
    create_summary_schema(&conn);
    append_summary(&conn, 1, "key-a", THREAD, true);
    let oversized = serde_json::json!({"model":"gpt-6.1-sol", "requestedServiceTier":"priority", "success":true,
        "outputTokens":200, "outputGenerationMs":1000, "inputTokens":1000, "cachedTokens":900,
        "unexpected": "x".repeat(20_000)}).to_string();
    conn.execute(
        "UPDATE request_display_summaries SET data_json=?1 WHERE log_id=1",
        [oversized],
    )
    .unwrap();
    assert!(read_selected(&conn, &selected("gpt-6.1-sol", "priority")).is_none());
}

struct TestData {
    _directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl TestData {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        Self {
            _directory: directory,
            previous,
        }
    }
}

impl Drop for TestData {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
    }
}

#[test]
fn output_speed_profile_resolution_accepts_optional_installation_but_requires_instance() {
    let _guard = crate::modules::test_support::env_lock().lock().unwrap();
    let data = TestData::new();
    let profile = tempfile::tempdir().unwrap();
    let config = r#"model_provider = "local"
[model_providers.local]
base_url = "http://127.0.0.1:58554/v1"
experimental_bearer_token = "synthetic-speed-token"
[model_providers.local.http_headers]
x-cockpit-instance-id = "instance-a"
"#;
    std::fs::write(profile.path().join("config.toml"), config).unwrap();
    // A real client may have this local file without sending it on the wire.
    std::fs::write(profile.path().join("installation_id"), "local-only-id").unwrap();
    std::fs::write(
        data._directory.path().join("codex_local_access.json"),
        r#"{"port":58554,"apiKeys":[{"id":"key-a","key":"synthetic-speed-token"}]}"#,
    )
    .unwrap();
    let conn = Connection::open(
        data._directory
            .path()
            .join("codex_local_access_logs.sqlite"),
    )
    .unwrap();
    create_schema(&conn);
    append(&conn, "current", Sample::default());
    drop(conn);

    let speed = read_output_speed(profile.path(), THREAD).unwrap().unwrap();
    assert_eq!(speed.tokens_per_second, 100.0);
    assert_eq!(speed.samples, 1);
    let explicit = format!("{config}x-codex-installation-id = \"explicit-installation\"\n");
    std::fs::write(profile.path().join("config.toml"), explicit).unwrap();
    assert!(read_output_speed(profile.path(), THREAD).unwrap().is_none());
    let conn = Connection::open(
        data._directory
            .path()
            .join("codex_local_access_logs.sqlite"),
    )
    .unwrap();
    conn.execute(
        "UPDATE request_diagnostics SET client_installation_id='explicit-installation'",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(read_output_speed(profile.path(), THREAD).unwrap().is_some());

    std::fs::write(
        profile.path().join("config.toml"),
        config.replace("x-cockpit-instance-id = \"instance-a\"", ""),
    )
    .unwrap();
    assert!(read_output_speed(profile.path(), THREAD).unwrap().is_none());
    std::fs::write(
        profile.path().join("config.toml"),
        config.replace("synthetic-speed-token", "unregistered-token"),
    )
    .unwrap();
    assert!(read_output_speed(profile.path(), THREAD).unwrap().is_none());
    std::fs::write(
        profile.path().join("config.toml"),
        config.replace("127.0.0.1", "gateway.example.test"),
    )
    .unwrap();
    assert!(read_output_speed(profile.path(), THREAD).unwrap().is_none());
    assert!(read_output_speed(profile.path(), "not-a-thread-id").is_err());
}

#[test]
fn output_speed_profile_selection_preserves_unknown_and_named_profile_overrides() {
    let profile = tempfile::tempdir().unwrap();
    std::fs::write(
        profile.path().join("config.toml"),
        "model = 'gpt-6.1-sol'\n",
    )
    .unwrap();
    let unknown = read_profile_selection(profile.path()).unwrap();
    assert_eq!(unknown.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(unknown.service_tier, None);
    std::fs::write(profile.path().join("config.toml"), "model='gpt-6.1-sol'\nservice_tier='auto'\nprofile='quick'\n[profiles.quick]\nmodel='gpt-6.1-sol-fast'\nservice_tier='fast'\n").unwrap();
    assert_eq!(
        read_profile_selection(profile.path()).unwrap(),
        selected("gpt-6.1-sol-fast", "priority")
    );
    assert_eq!(normalize_service_tier("high"), None);
    assert_eq!(normalize_service_tier("auto"), Some("auto".into()));
}

#[test]
fn output_speed_binding_is_semantic_and_invalidates_keys_or_database_replacement() {
    let _guard = crate::modules::test_support::env_lock().lock().unwrap();
    let data = TestData::new();
    let profile = tempfile::tempdir().unwrap();
    let base = "model_provider='local'\n[model_providers.local]\nbase_url='http://127.0.0.1:58554/v1'\nexperimental_bearer_token='synthetic-speed-token'\n[model_providers.local.http_headers]\nx-cockpit-instance-id='instance-a'\n";
    std::fs::write(
        profile.path().join("config.toml"),
        format!("model='gpt-6.1-sol'\nservice_tier='default'\n{base}"),
    )
    .unwrap();
    let service = data._directory.path().join("codex_local_access.json");
    std::fs::write(
        &service,
        r#"{"port":58554,"apiKeys":[{"id":"key-a","key":"synthetic-speed-token"}]}"#,
    )
    .unwrap();
    let db_path = data
        ._directory
        .path()
        .join("codex_local_access_logs.sqlite");
    std::fs::write(&db_path, "not-opened-by-fingerprint").unwrap();
    let first = binding_snapshot(profile.path()).unwrap().unwrap();
    assert_eq!(first.len(), 64);
    assert!(!first.contains("synthetic-speed-token"));
    std::fs::write(
        profile.path().join("config.toml"),
        format!("model='grok-4'\nservice_tier='priority'\nmodel_reasoning_effort='high'\n{base}"),
    )
    .unwrap();
    assert_eq!(
        binding_snapshot(profile.path()).unwrap().as_deref(),
        Some(first.as_str())
    );
    let mut volatile = serde_json::json!({"port":58554,"apiKeys":[
        {"id":"key-a","key":"synthetic-speed-token","tokenUsed":999,"updatedAt":200,"lastUsedAt":200},
        {"id":"unrelated","key":"other-token","enabled":true,"tokenUsed":123}
    ],"updatedAt":200,"modelPricingVersion":999,"modelPricings":[{"model":"gpt-6.1-sol","input":999}]});
    std::fs::write(&service, volatile.to_string()).unwrap();
    assert_eq!(
        binding_snapshot(profile.path()).unwrap().as_deref(),
        Some(first.as_str())
    );
    volatile["apiKeys"][1]["enabled"] = serde_json::json!(false);
    volatile["apiKeys"][1]["key"] = serde_json::json!("rotated-unrelated-token");
    std::fs::write(&service, volatile.to_string()).unwrap();
    assert_eq!(
        binding_snapshot(profile.path()).unwrap().as_deref(),
        Some(first.as_str())
    );
    for (field, value) in [
        ("enabled", serde_json::json!(false)),
        ("accountIds", serde_json::json!(["account-b"])),
        ("allowedModels", serde_json::json!(["gpt-6.1-sol"])),
        ("tokenLimit", serde_json::json!(1000)),
    ] {
        let mut permission = volatile.clone();
        permission["apiKeys"][0][field] = value;
        std::fs::write(&service, permission.to_string()).unwrap();
        assert_ne!(
            binding_snapshot(profile.path()).unwrap().as_deref(),
            Some(first.as_str())
        );
    }
    volatile["port"] = serde_json::json!(58555);
    std::fs::write(&service, volatile.to_string()).unwrap();
    assert_ne!(
        binding_snapshot(profile.path()).unwrap().as_deref(),
        Some(first.as_str())
    );
    std::fs::write(
        &service,
        r#"{"port":58554,"apiKeys":[{"id":"key-b","key":"synthetic-speed-token"}]}"#,
    )
    .unwrap();
    let changed_key = binding_snapshot(profile.path()).unwrap().unwrap();
    assert_ne!(changed_key, first);
    std::fs::rename(&db_path, data._directory.path().join("old.sqlite")).unwrap();
    std::fs::write(&db_path, "replacement").unwrap();
    assert_ne!(
        binding_snapshot(profile.path()).unwrap().unwrap(),
        changed_key
    );
}

#[test]
fn output_speed_locked_database_has_verified_binding_but_non_sqlite_failures_do_not() {
    let _guard = crate::modules::test_support::env_lock().lock().unwrap();
    let data = TestData::new();
    let profile = tempfile::tempdir().unwrap();
    std::fs::write(profile.path().join("config.toml"), "model_provider='local'\n[model_providers.local]\nbase_url='http://127.0.0.1:58554/v1'\nexperimental_bearer_token='synthetic-speed-token'\n[model_providers.local.http_headers]\nx-cockpit-instance-id='instance-a'\n").unwrap();
    let service = data._directory.path().join("codex_local_access.json");
    std::fs::write(
        &service,
        r#"{"port":58554,"apiKeys":[{"id":"key-a","key":"synthetic-speed-token"}]}"#,
    )
    .unwrap();
    let conn = Connection::open(
        data._directory
            .path()
            .join("codex_local_access_logs.sqlite"),
    )
    .unwrap();
    create_schema(&conn);
    append(
        &conn,
        "real",
        Sample {
            requested_tier: Some("priority"),
            ..Default::default()
        },
    );
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let locked = read_output_speed_with_selection(
        profile.path(),
        THREAD,
        &selected("gpt-6.1-sol", "priority"),
    );
    assert!(locked.result.is_err());
    assert!(locked.transient_failure);
    assert!(locked.binding.is_some());
    conn.execute_batch("ROLLBACK").unwrap();
    let recovered = read_output_speed_with_selection(
        profile.path(),
        THREAD,
        &selected("gpt-6.1-sol", "priority"),
    );
    assert!(recovered.result.unwrap().is_some());
    std::fs::write(&service, r#"{"port":58554,"apiKeys":[{"id":"key-a","key":"synthetic-speed-token","enabled":false}]}"#).unwrap();
    let disabled = read_output_speed_with_selection(
        profile.path(),
        THREAD,
        &selected("gpt-6.1-sol", "priority"),
    );
    assert!(disabled.result.unwrap().is_none());
    assert!(disabled.binding.is_none());
    assert!(!transient_query_error(
        "output-speed selection unavailable: invalid TOML"
    ));
    assert!(!transient_query_error("unable to open database file"));
}
