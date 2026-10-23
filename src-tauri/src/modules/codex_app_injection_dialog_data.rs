//! Read-only data for the injected usage dialog. Call only on a blocking worker.
//! No payload, authorization or response body is read or sent to the renderer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::recent_account::{open_gateway_logs, read_small_file, RequestScope};

#[path = "codex_app_injection_dialog_summary.rs"]
mod summaries;

const PAGE_SIZE: usize = 10;
const QUERY_BUDGET: Duration = Duration::from_millis(900);
const CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
// The expression is evaluated in SQLite before JSON parsing or Rust allocation.
const ATTEMPTS: &str = "CASE WHEN length(CAST(d.attempts_json AS BLOB))<=65536 \
    AND json_valid(d.attempts_json) THEN d.attempts_json ELSE '[]' END";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DialogQuery {
    pub request_token: String,
    pub thread_id: Option<String>,
    pub scope: String,
    pub period: String,
    pub cursor: Option<String>,
    pub view: String,
    #[serde(default)]
    pub binding: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    timestamp: i64,
    id: i64,
    snapshot_id: i64,
    binding: String,
}

pub(super) fn parse_query(value: &Value) -> Option<DialogQuery> {
    let mut query: DialogQuery = serde_json::from_value(value.clone()).ok()?;
    if query.request_token.is_empty()
        || query.request_token.len() > 128
        || !query
            .request_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        || !matches!(query.scope.as_str(), "chat" | "service")
        || !matches!(query.period.as_str(), "today" | "week")
        || !matches!(query.view.as_str(), "overview" | "logs")
        || query.binding.as_ref().is_some_and(|binding| {
            binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return None;
    }
    if let Some(thread) = &query.thread_id {
        query.thread_id = Some(uuid::Uuid::parse_str(thread).ok()?.hyphenated().to_string());
    }
    if let Some(cursor) = &query.cursor {
        if cursor.len() > 512 || decode_cursor(cursor).is_none() || query.view != "logs" {
            return None;
        }
    }
    Some(query)
}

fn decode_cursor(raw: &str) -> Option<Cursor> {
    let cursor: Cursor = serde_json::from_str(raw).ok()?;
    (cursor.id > 0
        && cursor.snapshot_id >= cursor.id
        && (0..=MAX_SAFE_INTEGER).contains(&cursor.timestamp)
        && cursor.binding.len() == 16
        && cursor.binding.bytes().all(|b| b.is_ascii_hexdigit()))
    .then_some(cursor)
}

fn response(query: &DialogQuery, available: bool) -> Value {
    json!({
        "requestToken": query.request_token, "threadId": query.thread_id,
        "scope": query.scope, "period": query.period, "cursor": query.cursor,
        "view": query.view, "binding": query.binding, "available": available,
        "requests": {"rows": [], "nextCursor": null},
        "summary": null, "session": null
    })
}

/// Scope is resolved from the selected profile's actual local gateway credentials.
/// Service scope deliberately includes every key belonging to that local service.
pub(super) fn read_dialog_data(profile: &Path, query: &DialogQuery) -> Result<Value, String> {
    if parse_query(&serde_json::to_value(query).map_err(|e| e.to_string())?).as_ref() != Some(query)
    {
        return Err("invalid dialog query".into());
    }
    let before = super::output_speed::binding_snapshot(profile)?;
    if query.binding.is_some() && before != query.binding {
        return Ok(unavailable(query, "binding_changed"));
    }
    let Some((conn, scope)) = open_gateway_logs(profile)? else {
        return Ok(unavailable(query, "service_unavailable"));
    };
    if !matches_local_service(profile, &conn, &scope)? {
        return Ok(unavailable(query, "service_unavailable"));
    }
    let cache_identity = cache_identity(profile, &conn, &scope)?;
    let mut output = with_budget(&conn, QUERY_BUDGET, |started| {
        read_data(&conn, &scope, query, started, Some(&cache_identity))
    })?;
    // All account metadata I/O is after the SQLite read transaction and timer
    // have finished. It cannot hold the request-log snapshot or its locks.
    if let Some(data_dir) = conn.path().and_then(|path| Path::new(path).parent()) {
        enrich_account_display_names(data_dir, &mut output);
        enrich_gpt_api_key_count(data_dir, &scope, query, &cache_identity, &mut output);
    }
    if super::output_speed::binding_snapshot(profile)? != before {
        return Ok(unavailable(query, "binding_changed"));
    }
    Ok(output)
}

fn unavailable(query: &DialogQuery, reason: &str) -> Value {
    let mut output = response(query, false);
    output["unavailableReason"] = json!(reason);
    output
}

pub(super) fn matches_local_service(
    profile: &Path,
    conn: &Connection,
    scope: &RequestScope,
) -> Result<bool, String> {
    let Some(path) = conn.path().and_then(|path| Path::new(path).parent()) else {
        return Ok(false);
    };
    let Some(config) = read_small_file(&path.join("codex_local_access.json"))? else {
        return Ok(false);
    };
    let config: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    if let Some(keys) = config["apiKeys"].as_array().filter(|keys| !keys.is_empty()) {
        if !keys.iter().any(|key| {
            key["id"].as_str() == Some(&scope.key_id) && key["enabled"].as_bool() != Some(false)
        }) {
            return Ok(false);
        }
    }
    let Some(port) = config["port"]
        .as_u64()
        .filter(|port| (1..=65535).contains(port))
    else {
        return Ok(false);
    };
    let Some((base_url, _, _)) = super::recent_account::profile_gateway(profile)? else {
        return Ok(false);
    };
    let url = reqwest::Url::parse(&base_url).ok();
    Ok(url
        .and_then(|url| url.port_or_known_default())
        .map(u64::from)
        == Some(port))
}

fn with_budget<T>(
    conn: &Connection,
    budget: Duration,
    operation: impl FnOnce(Instant) -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    let interrupt = conn.get_interrupt_handle();
    let (finished, completion) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|threads| {
        let _timer = std::thread::Builder::new()
            .name("codex-dialog-query-budget".into())
            .spawn_scoped(threads, move || {
                if matches!(
                    completion.recv_timeout(budget),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    interrupt.interrupt();
                }
            })
            .map_err(|error| format!("dialog query timer unavailable: {error}"))?;
        let result = operation(started);
        let _ = finished.send(());
        if started.elapsed() > budget {
            return Err("dialog query budget exceeded".into());
        }
        result
    })
}

fn check_budget(started: Instant) -> Result<(), String> {
    if started.elapsed() > QUERY_BUDGET {
        Err("dialog query budget exceeded".into())
    } else {
        Ok(())
    }
}

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())
}

struct Schema {
    logs: HashSet<String>,
}

fn schema(conn: &Connection) -> Result<Option<Schema>, String> {
    let logs = columns(conn, "request_logs")?;
    let diagnostics = columns(conn, "request_diagnostics")?;
    if ![
        "id",
        "timestamp",
        "request_id",
        "api_key_id",
        "client_instance_id",
        "success",
        "http_status",
        "latency_ms",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cached_tokens",
        "reasoning_tokens",
        "estimated_cost_usd",
    ]
    .iter()
    .all(|name| logs.contains(*name))
        || ![
            "request_id",
            "client_identity_captured",
            "client_thread_id",
            "client_identity_metadata_captured",
            "client_installation_id",
            "attempts_json",
            "failure_phase",
        ]
        .iter()
        .all(|name| diagnostics.contains(*name))
    {
        return Ok(None);
    }
    // Never migrate an old DB on this observational path.
    for index in [
        "idx_codex_local_access_logs_timestamp",
        "idx_codex_local_access_logs_api_key",
    ] {
        let found: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                [index],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !found {
            return Ok(None);
        }
    }
    Ok(Some(Schema { logs }))
}

fn upstream_predicate() -> String {
    format!(
        "EXISTS(SELECT 1 FROM json_each({ATTEMPTS}) AS a \
         WHERE json_extract(a.value,'$.accountId')<>'' \
         AND json_extract(a.value,'$.transport') NOT IN ('','preflight') \
         AND json_extract(a.value,'$.status') BETWEEN 100 AND 599 \
         AND COALESCE(json_extract(a.value,'$.failurePhase'),'') \
             NOT IN ('routing','identity','validation','preparation'))"
    )
}

fn historical_response_predicate() -> &'static str {
    // Only an absent diagnostic may use this evidence. Explicit preflight
    // diagnostics must never be overruled by an accidentally successful row.
    "d.request_id IS NULL AND l.success=1 AND l.http_status BETWEEN 200 AND 299 \
     AND (l.input_tokens>0 OR l.output_tokens>0 OR l.total_tokens>0)"
}

fn dispatched_predicate() -> String {
    format!(
        "COALESCE(({} OR ({})),0)",
        upstream_predicate(),
        historical_response_predicate()
    )
}

fn unsent_predicate() -> String {
    format!(
        "COALESCE(((length(CAST(d.attempts_json AS BLOB))<=65536 \
         AND json_valid(d.attempts_json) AND json_array_length({ATTEMPTS})=0 \
         AND d.failure_phase IN ('routing','identity','validation','preparation','selection')) \
         OR (EXISTS(SELECT 1 FROM json_each({ATTEMPTS})) \
         AND NOT EXISTS(SELECT 1 FROM json_each({ATTEMPTS}) AS a \
             WHERE COALESCE(json_extract(a.value,'$.transport'),'')<>'preflight' \
             AND COALESCE(json_extract(a.value,'$.failurePhase'),'') \
             NOT IN ('routing','identity','validation','preparation')))),0)"
    )
}

fn chat_available(scope: &RequestScope, thread: Option<&str>) -> bool {
    !scope.key_id.is_empty()
        && scope
            .instance_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
        && thread.is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

fn read_data(
    conn: &Connection,
    scope: &RequestScope,
    query: &DialogQuery,
    started: Instant,
    cache_identity: Option<&str>,
) -> Result<Value, String> {
    let fast = summaries::available(conn)?;
    let schema = if fast { None } else { schema(conn)? };
    if !fast && schema.is_none() {
        return Ok(unavailable(query, "schema_unavailable"));
    }
    let chat_valid = chat_available(scope, query.thread_id.as_deref());
    if query.view == "logs" && query.scope == "chat" && !chat_valid {
        return Ok(unavailable(query, "unlinked_session"));
    }
    let mut output = response(query, true);
    // One read transaction keeps page rows, latest session and aggregate consistent.
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let snapshot_id: i64 = tx
        .query_row("SELECT COALESCE(MAX(id),0) FROM request_logs", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    if query.view == "logs" {
        output["requests"] = if fast {
            summaries::read_page(&tx, scope, query, snapshot_id, PAGE_SIZE, started)?
        } else {
            read_page(
                &tx,
                schema.as_ref().unwrap(),
                scope,
                query,
                snapshot_id,
                PAGE_SIZE,
                started,
            )?
        };
    }
    if chat_valid && query.view == "overview" {
        let mut session_query = query.clone();
        session_query.scope = "chat".into();
        session_query.cursor = None;
        let session_page = if fast {
            summaries::read_page(&tx, scope, &session_query, snapshot_id, 1, started)?
        } else {
            read_page(
                &tx,
                schema.as_ref().unwrap(),
                scope,
                &session_query,
                snapshot_id,
                1,
                started,
            )?
        };
        if let Some(latest) = session_page["rows"]
            .as_array()
            .and_then(|rows| rows.first())
        {
            output["session"] = json!({
                "threadId": query.thread_id,
                "model": latest["model"],
                "requestedModel": latest["requestedModel"],
                "upstreamModel": latest["upstreamModel"],
                "firstResponseMs": latest["firstResponseMs"],
                "reasoningEffort": latest["reasoningEffort"]
            });
        }
    }
    if query.view == "overview" {
        let until = chrono::Utc::now().timestamp_millis();
        let since = period_start(&query.period, until)?;
        let key = cache_identity.map(|identity| format!("{identity}:{}:{since}", query.period));
        output["summary"] = if let Some(cached) = key.as_deref().and_then(cached_summary) {
            cached
        } else {
            let summary = read_summary(&tx, since, until, started)?;
            if let Some(key) = key {
                remember_summary(key, summary.clone());
            }
            summary
        };
    }
    tx.commit().map_err(|e| e.to_string())?;
    check_budget(started)?;
    Ok(output)
}

fn cursor_binding(scope: &RequestScope, query: &DialogQuery) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    scope.key_id.hash(&mut hash);
    scope.instance_id.hash(&mut hash);
    scope.installation_id.hash(&mut hash);
    query.thread_id.hash(&mut hash);
    query.scope.hash(&mut hash);
    query.period.hash(&mut hash);
    query.binding.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn projection(schema: &Schema, name: &str, limit: usize) -> String {
    if schema.logs.contains(name) {
        format!("substr(l.{name},1,{limit}) AS {name}")
    } else {
        format!("NULL AS {name}")
    }
}

fn read_page(
    conn: &Connection,
    schema: &Schema,
    scope: &RequestScope,
    query: &DialogQuery,
    snapshot_id: i64,
    limit: usize,
    started: Instant,
) -> Result<Value, String> {
    let binding = cursor_binding(scope, query);
    let cursor = query
        .cursor
        .as_deref()
        .map(|raw| {
            decode_cursor(raw)
                .filter(|cursor| cursor.binding == binding)
                .ok_or_else(|| "dialog cursor scope changed".to_string())
        })
        .transpose()?;
    let snapshot_id = cursor
        .as_ref()
        .map_or(snapshot_id, |cursor| cursor.snapshot_id);
    let timestamp = cursor
        .as_ref()
        .map_or(MAX_SAFE_INTEGER, |cursor| cursor.timestamp);
    let id = cursor.as_ref().map_or(i64::MAX, |cursor| cursor.id);
    let is_chat = query.scope == "chat";
    let index = if is_chat {
        "idx_codex_local_access_logs_api_key"
    } else {
        "idx_codex_local_access_logs_timestamp"
    };
    let identity = if is_chat {
        "AND l.api_key_id=:key AND l.client_instance_id=:instance \
         AND d.client_identity_captured=1 \
         AND d.client_identity_metadata_captured=1 \
         AND lower(d.client_thread_id)=:thread \
         AND (:installation IS NULL OR (d.client_identity_metadata_captured=1 \
             AND d.client_installation_id=:installation))"
    } else {
        // Keep a single set of named bound parameters for both query shapes.
        "AND (:key IS NOT NULL) AND (:instance IS NULL OR 1) \
         AND (:thread IS NULL OR 1) AND (:installation IS NULL OR 1)"
    };
    let mut selected = [
        "request_id",
        "account_id",
        "email",
        "model_id",
        "requested_model",
        "upstream_model",
        "reasoning_effort",
        "error_category",
        "authorization_route",
        "service_tier",
        "requested_service_tier",
        "response_service_tier",
    ]
    .iter()
    .map(|name| projection(schema, name, 256))
    .collect::<Vec<_>>();
    for name in ["http_status", "first_response_ms", "output_generation_ms"] {
        selected.push(if schema.logs.contains(name) {
            format!("l.{name}")
        } else {
            format!("NULL AS {name}")
        });
    }
    let sql = format!(
        "SELECT l.id,l.timestamp,l.success,l.latency_ms,l.input_tokens,l.output_tokens,\
         l.total_tokens,l.cached_tokens,{}, {ATTEMPTS} AS attempts \
         FROM request_logs AS l INDEXED BY {index} \
         LEFT JOIN request_diagnostics AS d ON d.request_id=l.request_id \
         WHERE l.id<=:snapshot AND l.timestamp<=:timestamp \
         AND (l.timestamp<:timestamp OR l.id<:id) {identity} AND {} \
         ORDER BY l.timestamp DESC,l.id DESC LIMIT :limit",
        selected.join(","),
        dispatched_predicate()
    );
    let mut statement = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = statement
        .query(rusqlite::named_params! {
            ":snapshot": snapshot_id, ":timestamp": timestamp, ":id": id,
            ":key": scope.key_id, ":instance": scope.instance_id,
            ":thread": query.thread_id, ":installation": scope.installation_id,
            ":limit": limit as i64 + 1,
        })
        .map_err(|e| e.to_string())?;
    let mut values = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_budget(started)?;
        values.push(read_row(row)?);
    }
    let more = values.len() > limit;
    values.truncate(limit);
    let next_cursor = if more {
        values
            .last()
            .map(|last| {
                serde_json::to_string(&Cursor {
                    timestamp: last["timestamp"].as_i64().unwrap_or_default(),
                    id: last["id"]
                        .as_str()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_default(),
                    snapshot_id,
                    binding,
                })
                .map_err(|e| e.to_string())
            })
            .transpose()?
    } else {
        None
    };
    Ok(json!({"rows": values, "nextCursor": next_cursor}))
}

fn text(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    })
}

fn read_row(row: &rusqlite::Row<'_>) -> Result<Value, String> {
    crate::modules::codex_request_summary::display_row(row)
}

fn period_start(period: &str, until: i64) -> Result<i64, String> {
    let local = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(until)
        .ok_or_else(|| "invalid dialog time".to_string())?
        .with_timezone(&Local);
    let day = if period == "week" {
        local.date_naive().checked_sub_days(chrono::Days::new(6))
    } else {
        Some(local.date_naive())
    }
    .ok_or_else(|| "invalid dialog date".to_string())?;
    let midnight = day.and_hms_opt(0, 0, 0).ok_or("invalid dialog midnight")?;
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|time| time.timestamp_millis())
        .ok_or_else(|| "local dialog midnight unavailable".into())
}

fn read_summary(
    conn: &Connection,
    since: i64,
    until: i64,
    started: Instant,
) -> Result<Value, String> {
    let sql = format!(
        "WITH evidence AS (SELECT l.success,l.input_tokens,l.output_tokens,l.total_tokens,\
         l.cached_tokens,l.reasoning_tokens,l.estimated_cost_usd,l.latency_ms,\
         ({}) AS dispatched,({}) AS unsent \
         FROM request_logs AS l INDEXED BY idx_codex_local_access_logs_timestamp \
         LEFT JOIN request_diagnostics AS d ON d.request_id=l.request_id \
         WHERE l.timestamp>=?1 AND l.timestamp<=?2) \
         SELECT COUNT(*) FILTER(WHERE dispatched),\
         COALESCE(SUM(success=1) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(MAX(input_tokens,0)) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(MAX(output_tokens,0)) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(MAX(total_tokens,0)) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(MAX(cached_tokens,0)) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(MAX(reasoning_tokens,0)) FILTER(WHERE dispatched),0),\
         COALESCE(SUM(CASE WHEN estimated_cost_usd>0 THEN estimated_cost_usd ELSE 0 END) \
             FILTER(WHERE dispatched),0),\
         AVG(CASE WHEN success=1 THEN MAX(latency_ms,0) END) FILTER(WHERE dispatched),\
         COUNT(*) FILTER(WHERE NOT dispatched AND NOT unsent) FROM evidence",
        dispatched_predicate(),
        unsent_predicate()
    );
    let summary = conn
        .query_row(&sql, params![since, until], |row| {
            let count: i64 = row.get(0)?;
            let success: i64 = row.get(1)?;
            let cost: f64 = row.get(7)?;
            let latency: Option<f64> = row.get(8)?;
            let unknown: i64 = row.get(9)?;
            Ok(json!({
                "requests": count, "successRequests": success,
                "failureRequests": count.saturating_sub(success),
                "inputTokens": row.get::<_, i64>(2)?,
                "outputTokens": row.get::<_, i64>(3)?,
                "totalTokens": row.get::<_, i64>(4)?,
                "cachedTokens": row.get::<_, i64>(5)?,
                "reasoningTokens": row.get::<_, i64>(6)?,
                "estimatedCostUsd": cost.is_finite().then_some(cost),
                "averageLatencyMs": latency.filter(|value| value.is_finite()),
            "excludedUnknownRequests": unknown, "incomplete": unknown > 0,
                "sinceMs": since, "untilMs": until
            }))
        })
        .map_err(|e| e.to_string())?;
    check_budget(started)?;
    Ok(summary)
}

struct CachedSummary {
    key: String,
    at: Instant,
    value: Value,
}
static SUMMARY_CACHE: LazyLock<Mutex<VecDeque<CachedSummary>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

fn cached_summary(key: &str) -> Option<Value> {
    SUMMARY_CACHE
        .lock()
        .ok()?
        .iter()
        .find(|entry| entry.key == key && entry.at.elapsed() < CACHE_TTL)
        .map(|entry| entry.value.clone())
}

fn remember_summary(key: String, value: Value) {
    if let Ok(mut cache) = SUMMARY_CACHE.lock() {
        cache.retain(|entry| entry.key != key && entry.at.elapsed() < CACHE_TTL);
        cache.push_back(CachedSummary {
            key,
            at: Instant::now(),
            value,
        });
        while cache.len() > 8 {
            cache.pop_front();
        }
    }
}

fn cache_identity(
    profile: &Path,
    conn: &Connection,
    scope: &RequestScope,
) -> Result<String, String> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    profile.hash(&mut hash);
    scope.key_id.hash(&mut hash);
    scope.instance_id.hash(&mut hash);
    scope.installation_id.hash(&mut hash);
    let (_, bearer, _) = super::recent_account::profile_gateway(profile)?
        .ok_or("dialog profile binding unavailable")?;
    bearer.hash(&mut hash);
    let path = conn.path().ok_or("dialog database path unavailable")?;
    path.hash(&mut hash);
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.dev().hash(&mut hash);
        metadata.ino().hash(&mut hash);
    }
    metadata.created().ok().hash(&mut hash);
    if let Some(parent) = Path::new(path).parent() {
        let raw = read_small_file(&parent.join("codex_local_access.json"))?
            .ok_or("dialog service binding unavailable")?;
        let service: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        super::output_speed::service_binding_fields(&service, &bearer)
            .ok_or("dialog service key unavailable")?
            .to_string()
            .hash(&mut hash);
    }
    Ok(format!("{:016x}", hash.finish()))
}

fn enrich_gpt_api_key_count(
    data_dir: &Path,
    scope: &RequestScope,
    query: &DialogQuery,
    cache_identity: &str,
    output: &mut Value,
) {
    if query.view == "overview" && output["available"].as_bool() == Some(true) {
        output["gptApiKeyCount"] = json!(read_gpt_api_key_count(
            data_dir,
            &scope.key_id,
            cache_identity
        ));
    }
}

fn read_gpt_api_key_count(data_dir: &Path, key_id: &str, cache_identity: &str) -> Option<i64> {
    let cache_key = format!("gpt-api-key-count:{cache_identity}:{key_id}");
    if let Some(count) = cached_summary(&cache_key).and_then(|value| value.as_i64()) {
        return Some(count);
    }
    // Reuse the bounded display cache, with a distinct namespace. The key
    // already includes profile, bearer key identity, config and DB identity.
    // Missing or temporarily unreadable account metadata is never cached as 0.
    let raw = read_small_file(&data_dir.join("codex_local_access.json")).ok()??;
    let collection =
        serde_json::from_str::<crate::models::codex_local_access::CodexLocalAccessCollection>(&raw)
            .ok()?;
    let count = crate::modules::codex_local_access::injection_gpt_api_key_account_count(
        &collection,
        key_id,
        data_dir,
    )?;
    remember_summary(cache_key, json!(count));
    Some(count)
}

/// The plaintext index contains only id/email/plan, not custom account names or
/// provider hosts. Deserialize only this allowlist from the selected detail;
/// unknown fields (including all tokens and API keys) are ignored by serde.
#[derive(Deserialize)]
struct DisplayIdentity {
    id: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    auth_mode: Option<crate::models::codex::CodexAuthMode>,
    account_name: Option<String>,
    api_base_url: Option<String>,
    #[serde(default)]
    api_provider_mode: Option<crate::models::codex::CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
}

fn safe_display_name(value: Option<String>) -> Option<String> {
    text(value.map(|value| value.trim().to_string())).filter(|value| {
        let lower = value.trim().to_ascii_lowercase();
        let generated_api_name = lower.strip_prefix("api-key-").is_some_and(|suffix| {
            suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        !generated_api_name
            && !(lower.starts_with("codex_") && !lower.contains('@'))
            && !(lower.starts_with("chatgpt_plan_") && !lower.contains('@'))
    })
}

fn display_identity_name(identity: DisplayIdentity) -> Option<String> {
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexAuthMode, CodexTokens};
    // Independent ChatGPT authorization represents a person even when older
    // storage used the API-key shape, matching the API page's TS resolver.
    if identity.api_provider_id.as_deref() == Some("cockpit_chatgpt_plan")
        || identity.id.starts_with("chatgpt_plan_")
    {
        return safe_display_name(Some(identity.email))
            .or_else(|| safe_display_name(identity.account_name));
    }
    // API-page labels give a user's explicit name precedence for OAuth too.
    if let Some(name) = safe_display_name(identity.account_name.clone()) {
        return Some(name);
    }
    let auth_mode = identity.auth_mode.unwrap_or_else(|| {
        let generated = |value: &str| {
            value
                .trim()
                .to_ascii_lowercase()
                .strip_prefix("api-key-")
                .is_some_and(|suffix| {
                    suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        };
        // Legacy display identity only; this does not change auth or routing.
        if identity.id.starts_with("codex_apikey_")
            || generated(&identity.id)
            || generated(&identity.email)
            || identity.account_name.as_deref().is_some_and(generated)
        {
            CodexAuthMode::Apikey
        } else {
            CodexAuthMode::OAuth
        }
    });
    let mut account = CodexAccount::new(
        identity.id,
        identity.email,
        CodexTokens {
            id_token: String::new(),
            access_token: String::new(),
            refresh_token: None,
        },
    );
    account.auth_mode = auth_mode;
    account.account_name = identity.account_name;
    account.api_base_url = identity.api_base_url;
    account.api_provider_mode = identity
        .api_provider_mode
        .unwrap_or(CodexApiProviderMode::Custom);
    account.api_provider_name = identity.api_provider_name;
    safe_display_name(Some(account.display_name("")))
}

fn safe_account_filename(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn read_display_identity(data_dir: &Path, id: &str) -> Option<String> {
    if !safe_account_filename(id) {
        return None;
    }
    let path = data_dir.join("codex_accounts").join(format!("{id}.json"));
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    // Avoid directories, symlinks and abnormally large imported records. This
    // helper never scans the account directory or follows index repair paths.
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    let identity = crate::modules::secure_account_storage::read_account_file_readonly::<
        DisplayIdentity,
    >(&path, &data_dir.join("secure-account-storage.key"))
    .ok()?;
    if identity.id != id {
        return None;
    }
    display_identity_name(identity)
}

fn enrich_account_display_names(data_dir: &Path, output: &mut Value) {
    let Some(rows) = output["requests"]["rows"].as_array_mut() else {
        return;
    };
    let started = Instant::now();
    let mut names = HashMap::<String, Option<String>>::new();
    for row in rows.iter().take(PAGE_SIZE) {
        if started.elapsed() >= Duration::from_millis(100) {
            break;
        }
        let Some(id) = row["accountId"].as_str() else {
            continue;
        };
        names
            .entry(id.to_string())
            .or_insert_with(|| read_display_identity(data_dir, id));
    }
    for row in rows {
        let name = row["accountId"]
            .as_str()
            .and_then(|id| names.get(id))
            .and_then(Clone::clone)
            .or_else(|| safe_display_name(row["accountEmail"].as_str().map(str::to_owned)));
        row["accountDisplayName"] = json!(name);
    }
}

#[cfg(test)]
#[path = "codex_app_injection_dialog_data_tests.rs"]
mod tests;
