//! Small, write-time projections for the injected request list.
//!
//! This module never walks historical logs. Live usage/diagnostic writers call
//! `refresh_request` after persisting one request, under their existing write
//! lock. Detail and usage can arrive in either order. Raw bodies, credentials,
//! error messages and diagnostic JSON never enter the projection table.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::models::codex_local_access::CodexLocalAccessServiceTierAdaptation;

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_ATTEMPTS_BYTES: usize = 65_536;
const MAX_ATTEMPTS: usize = 32;
const MAX_DISPLAY_BYTES: usize = 16 * 1024;
// A normal request has one usage row. Keep malformed/reused IDs bounded too.
const MAX_REQUEST_ROWS: usize = 128;

pub(crate) fn ensure_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_display_summaries (
         log_id INTEGER PRIMARY KEY,request_id TEXT NOT NULL,timestamp INTEGER NOT NULL,
         api_key_id TEXT NOT NULL,client_instance_id TEXT NOT NULL,
         client_thread_id TEXT,client_installation_id TEXT,
         client_identity_captured INTEGER NOT NULL DEFAULT 0,
         client_identity_metadata_captured INTEGER NOT NULL DEFAULT 0,
         dispatched INTEGER NOT NULL DEFAULT 0,data_json TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS idx_request_display_chat ON request_display_summaries
            (api_key_id,client_instance_id,client_thread_id,timestamp DESC,log_id DESC);
         CREATE INDEX IF NOT EXISTS idx_request_display_timestamp ON request_display_summaries
            (timestamp DESC,log_id DESC);
         CREATE INDEX IF NOT EXISTS idx_request_display_request ON request_display_summaries
            (request_id);",
    )
    .map_err(|error| format!("request summary schema unavailable: {error}"))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

/// Called inside the request-log clear transaction; does not create a new table.
pub(crate) fn clear_if_present(conn: &Connection) -> Result<(), String> {
    if table_exists(conn, "request_display_summaries")? {
        conn.execute("DELETE FROM request_display_summaries", [])
            .map_err(|error| format!("request summary clear failed: {error}"))?;
    }
    Ok(())
}

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string())
}

fn text(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    })
}

fn safe_display_name(value: Option<String>) -> Option<String> {
    text(value.map(|value| value.trim().to_string())).filter(|value| {
        let lower = value.to_ascii_lowercase();
        let generated_api_name = lower.strip_prefix("api-key-").is_some_and(|suffix| {
            suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        !generated_api_name
            && !(lower.starts_with("codex_") && !lower.contains('@'))
            && !(lower.starts_with("chatgpt_plan_") && !lower.contains('@'))
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attempt {
    #[serde(default)]
    sequence: u32,
    #[serde(default)]
    started_at_ms: i64,
    #[serde(default)]
    account_id: String,
    account_email: Option<String>,
    authorization_route: Option<String>,
    authorization_fallback: Option<AuthorizationFallback>,
    service_tier_adaptation: Option<CodexLocalAccessServiceTierAdaptation>,
    #[serde(default)]
    transport: String,
    status: Option<u16>,
    failure_phase: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationFallback {
    #[serde(default)]
    from_route: String,
    #[serde(default)]
    to_route: String,
    #[serde(default)]
    from_account_id: String,
    #[serde(default)]
    to_account_id: String,
    #[serde(default)]
    reason: String,
}

impl AuthorizationFallback {
    fn is_recorded_same_account_fallback(&self) -> bool {
        self.from_route == "official"
            && self.to_route == "local_gateway"
            && !self.from_account_id.trim().is_empty()
            && self.from_account_id == self.to_account_id
            && !self.reason.trim().is_empty()
    }
}

fn authorization_route(value: Option<String>) -> Option<String> {
    value.filter(|value| matches!(value.as_str(), "official" | "local_gateway"))
}

fn parse_attempts(raw: &str) -> Vec<Attempt> {
    if raw.len() > MAX_ATTEMPTS_BYTES {
        return Vec::new();
    }
    serde_json::from_str::<Vec<Attempt>>(raw)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_ATTEMPTS)
        .collect()
}

fn actual_attempts(raw: &str) -> Vec<Attempt> {
    let mut attempts = parse_attempts(raw);
    attempts.retain(|attempt| {
        !attempt.account_id.trim().is_empty()
            && !attempt.transport.is_empty()
            && attempt.transport != "preflight"
            && attempt
                .status
                .is_some_and(|status| (100..=599).contains(&status))
            && !matches!(
                attempt.failure_phase.as_deref(),
                Some("routing" | "identity" | "validation" | "preparation")
            )
    });
    attempts.sort_by_key(|attempt| (attempt.sequence, attempt.started_at_ms));
    attempts
}

/// Shared with the API Service reader: explicit final-route evidence only.
pub(crate) fn service_tier_adaptation(raw: &str) -> Option<CodexLocalAccessServiceTierAdaptation> {
    let attempt = parse_attempts(raw)
        .into_iter()
        .filter(|attempt| attempt.authorization_route.is_some() && attempt.transport != "preflight")
        .last()?;
    let trace = attempt.service_tier_adaptation?;
    (attempt.authorization_route.as_deref() == Some("official")
        && trace.reason == "official_default_only"
        && trace.sent_tier == "default"
        && !trace.requested_tier.is_empty()
        && trace.requested_tier.len() <= 64
        && trace
            .requested_tier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        && !matches!(trace.requested_tier.as_str(), "auto" | "default"))
    .then_some(trace)
}

fn numeric(row: &rusqlite::Row<'_>, name: &str, positive: bool) -> rusqlite::Result<Option<i64>> {
    Ok(row
        .get::<_, Option<i64>>(name)?
        .filter(|value| (if positive { 1 } else { 0 }..=MAX_SAFE_INTEGER).contains(value)))
}

fn optional_text(row: &rusqlite::Row<'_>, name: &str) -> rusqlite::Result<Option<String>> {
    match row.get::<_, Option<String>>(name) {
        Ok(value) => Ok(text(value)),
        Err(rusqlite::Error::InvalidColumnName(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Both historical read-only fallback and write-time summaries use this exact
/// allowlist. No account file I/O or mutable current-plan lookup happens here.
pub(crate) fn display_row(row: &rusqlite::Row<'_>) -> Result<Value, String> {
    let read = || -> rusqlite::Result<Value> {
        let raw = row.get::<_, String>("attempts")?;
        let attempts = actual_attempts(&raw);
        let actual = attempts.last();
        let logged_account = row.get::<_, Option<String>>("account_id")?;
        let account = actual
            .and_then(|attempt| text(Some(attempt.account_id.clone())))
            .or_else(|| text(logged_account.clone()));
        let email = actual
            .and_then(|attempt| text(attempt.account_email.clone()))
            .or_else(|| {
                (logged_account == account)
                    .then(|| row.get::<_, Option<String>>("email").ok().flatten())
                    .flatten()
                    .and_then(|value| text(Some(value)))
            });
        let success = row.get::<_, i64>("success")? == 1;
        let route = actual
            .and_then(|attempt| authorization_route(attempt.authorization_route.clone()))
            .or(authorization_route(row.get("authorization_route")?));
        let adaptation = (route.as_deref() == Some("official"))
            .then(|| service_tier_adaptation(&raw))
            .flatten();
        Ok(json!({
            "id": row.get::<_, i64>("id")?.to_string(),
            "requestId": text(row.get("request_id")?),
            "timestamp": numeric(row,"timestamp",false)?,
            "accountDisplayName": safe_display_name(email.clone()),
            "accountId": account, "accountEmail": email,
            "plan": null,
            "model": text(row.get("model_id")?),
            "requestedModel": text(row.get("requested_model")?),
            "upstreamModel": text(row.get("upstream_model")?),
            "reasoningEffort": text(row.get("reasoning_effort")?),
            "requestedServiceTier": optional_text(row,"requested_service_tier")?,
            "responseServiceTier": optional_text(row,"response_service_tier")?,
            "serviceTier": optional_text(row,"service_tier")?,
            "serviceTierAdaptation": adaptation,
            "transport": actual.and_then(|attempt| text(Some(attempt.transport.clone()))),
            "authorizationRoute": route, "success": success,
            "httpStatus": numeric(row,"http_status",true)?,
            "errorCategory": text(row.get("error_category")?),
            "errorMessage": null,
            "inputTokens": numeric(row,"input_tokens",false)?,
            "outputTokens": numeric(row,"output_tokens",false)?,
            "cachedTokens": numeric(row,"cached_tokens",false)?,
            "totalTokens": numeric(row,"total_tokens",false)?,
            "firstResponseMs": numeric(row,"first_response_ms",false)?,
            "latencyMs": numeric(row,"latency_ms",false)?,
            "outputGenerationMs": numeric(row,"output_generation_ms",true)?,
            "recovered": success && attempts.iter().any(|attempt| attempt
                .authorization_fallback.as_ref()
                .is_some_and(AuthorizationFallback::is_recorded_same_account_fallback))
        }))
    };
    let data = read().map_err(|error| error.to_string())?;
    encode_display_data(&data)?;
    Ok(data)
}

fn encode_display_data(data: &Value) -> Result<String, String> {
    let encoded = serde_json::to_string(data).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_DISPLAY_BYTES {
        return Err("request summary display limit exceeded".into());
    }
    Ok(encoded)
}

fn projection(columns: &HashSet<String>, alias: &str, name: &str, is_text: bool) -> String {
    if columns.contains(name) {
        if is_text {
            format!("substr({alias}.{name},1,256) AS {name}")
        } else {
            format!("{alias}.{name}")
        }
    } else {
        format!("NULL AS {name}")
    }
}

struct Summary {
    id: i64,
    request_id: String,
    timestamp: i64,
    key: String,
    instance: String,
    thread: Option<String>,
    installation: Option<String>,
    identity: bool,
    metadata: bool,
    dispatched: bool,
    data: Value,
}

fn retain_pruned_evidence(conn: &Connection, summary: &mut Summary) -> Result<(), String> {
    let previous = conn
        .query_row(
            "SELECT client_thread_id,client_installation_id,client_identity_captured,
         client_identity_metadata_captured,dispatched,data_json
         FROM request_display_summaries WHERE log_id=?1
         AND length(CAST(data_json AS BLOB))<=16384",
            [summary.id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((thread, installation, identity, metadata, dispatched, encoded)) = previous else {
        return Ok(());
    };
    // Diagnostic retention is independent from request-log retention. A replayed
    // usage update must not erase already-recorded identity/final-route evidence,
    // or turn an explicitly unsent request into a legacy dispatched request.
    let previous: Value = serde_json::from_str(&encoded).map_err(|error| error.to_string())?;
    summary.thread = thread;
    summary.installation = installation;
    summary.identity = identity;
    summary.metadata = metadata;
    summary.dispatched = dispatched;
    for name in [
        "accountId",
        "accountEmail",
        "accountDisplayName",
        "authorizationRoute",
        "transport",
        "recovered",
        "serviceTierAdaptation",
    ] {
        summary.data[name] = previous[name].clone();
    }
    Ok(())
}

fn pending_rows(conn: &Connection, request_id: &str) -> Result<Vec<Summary>, String> {
    let logs = columns(conn, "request_logs")?;
    let diagnostics = columns(conn, "request_diagnostics")?;
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
        "requested_service_tier",
        "response_service_tier",
        "service_tier",
        "api_key_id",
        "client_instance_id",
    ]
    .iter()
    .map(|name| projection(&logs, "l", name, true))
    .collect::<Vec<_>>();
    for name in ["http_status", "first_response_ms", "output_generation_ms"] {
        selected.push(projection(&logs, "l", name, false));
    }
    for name in [
        "client_identity_captured",
        "client_identity_metadata_captured",
    ] {
        selected.push(projection(&diagnostics, "d", name, false));
    }
    for name in ["client_thread_id", "client_installation_id"] {
        selected.push(projection(&diagnostics, "d", name, true));
    }
    let join = if diagnostics.is_empty() {
        selected.push("NULL AS diagnostic_request_id".into());
        selected.push("'[]' AS attempts".into());
        ""
    } else {
        selected.push("d.request_id AS diagnostic_request_id".into());
        selected.push(
            "CASE WHEN length(CAST(d.attempts_json AS BLOB))<=65536 \
            AND json_valid(d.attempts_json) THEN d.attempts_json ELSE '[]' END AS attempts"
                .into(),
        );
        "LEFT JOIN request_diagnostics AS d ON d.request_id=l.request_id"
    };
    let sql = format!(
        "SELECT l.id,l.timestamp,l.success,l.latency_ms,l.input_tokens,l.output_tokens,\
         l.total_tokens,l.cached_tokens,{} FROM request_logs AS l \
         INDEXED BY idx_codex_local_access_logs_request_id {join} \
         WHERE l.request_id=?1 ORDER BY l.timestamp DESC,l.id DESC LIMIT ?2",
        selected.join(",")
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![request_id, MAX_REQUEST_ROWS as i64 + 1])
        .map_err(|error| error.to_string())?;
    let mut pending = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        if pending.len() == MAX_REQUEST_ROWS {
            return Err("request summary row limit exceeded".into());
        }
        let data = display_row(row)?;
        let thread = text(
            row.get("client_thread_id")
                .map_err(|error| error.to_string())?,
        )
        .and_then(|value| uuid::Uuid::parse_str(&value).ok())
        .map(|value| value.hyphenated().to_string());
        let identity = row
            .get::<_, Option<i64>>("client_identity_captured")
            .map_err(|error| error.to_string())?
            == Some(1)
            && thread.is_some();
        let metadata = row
            .get::<_, Option<i64>>("client_identity_metadata_captured")
            .map_err(|error| error.to_string())?
            == Some(1);
        let raw: String = row.get("attempts").map_err(|error| error.to_string())?;
        let diagnostic_missing = row
            .get::<_, Option<String>>("diagnostic_request_id")
            .map_err(|error| error.to_string())?
            .is_none();
        let historical_response = diagnostic_missing
            && data["success"] == true
            && data["httpStatus"]
                .as_i64()
                .is_some_and(|status| (200..=299).contains(&status))
            && ["inputTokens", "outputTokens", "totalTokens"]
                .iter()
                .any(|name| data[name].as_i64().is_some_and(|tokens| tokens > 0));
        let id = row.get::<_, i64>("id").map_err(|error| error.to_string())?;
        let timestamp = data["timestamp"]
            .as_i64()
            .ok_or("invalid request summary timestamp")?;
        if id <= 0 {
            return Err("invalid request summary id".into());
        }
        let mut summary = Summary {
            id,
            request_id: request_id.to_string(),
            timestamp,
            key: text(row.get("api_key_id").map_err(|error| error.to_string())?)
                .unwrap_or_default(),
            instance: text(
                row.get("client_instance_id")
                    .map_err(|error| error.to_string())?,
            )
            .unwrap_or_default(),
            thread,
            installation: text(
                row.get("client_installation_id")
                    .map_err(|error| error.to_string())?,
            ),
            identity,
            metadata,
            dispatched: !actual_attempts(&raw).is_empty() || historical_response,
            data,
        };
        if diagnostic_missing {
            retain_pruned_evidence(conn, &mut summary)?;
        }
        pending.push(summary);
    }
    Ok(pending)
}

/// Refreshes only this already-persisted request. A failed projection never
/// rolls back the original log/diagnostic; the next live update can retry it.
pub(crate) fn refresh_request(conn: &Connection, request_id: &str) -> Result<(), String> {
    let request_id = request_id.trim();
    if text(Some(request_id.to_string())).is_none() {
        return Ok(());
    }
    ensure_schema(conn)?;
    let pending = pending_rows(conn, request_id)?;
    conn.execute_batch("SAVEPOINT cockpit_request_summary")
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let mut upsert = conn.prepare_cached(
            "INSERT INTO request_display_summaries
             (log_id,request_id,timestamp,api_key_id,client_instance_id,client_thread_id,
              client_installation_id,client_identity_captured,client_identity_metadata_captured,
              dispatched,data_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(log_id) DO UPDATE SET request_id=excluded.request_id,
              timestamp=excluded.timestamp,api_key_id=excluded.api_key_id,
              client_instance_id=excluded.client_instance_id,client_thread_id=excluded.client_thread_id,
              client_installation_id=excluded.client_installation_id,
              client_identity_captured=excluded.client_identity_captured,
              client_identity_metadata_captured=excluded.client_identity_metadata_captured,
              dispatched=excluded.dispatched,data_json=excluded.data_json",
        ).map_err(|error| error.to_string())?;
        for summary in pending {
            let encoded = encode_display_data(&summary.data)?;
            upsert
                .execute(params![
                    summary.id,
                    summary.request_id,
                    summary.timestamp,
                    summary.key,
                    summary.instance,
                    summary.thread,
                    summary.installation,
                    summary.identity as i64,
                    summary.metadata as i64,
                    summary.dispatched as i64,
                    encoded
                ])
                .map_err(|error| error.to_string())?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        let _ = conn
            .execute_batch("ROLLBACK TO cockpit_request_summary; RELEASE cockpit_request_summary");
        return Err(error);
    }
    conn.execute_batch("RELEASE cockpit_request_summary")
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "codex_request_summary_tests.rs"]
mod tests;
