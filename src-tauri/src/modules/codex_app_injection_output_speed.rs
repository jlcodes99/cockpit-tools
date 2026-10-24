//! Per-thread throughput from recorded generation timing. All configuration,
//! SQLite and metadata reads belong on the caller's bounded blocking worker.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::recent_account::{
    open_gateway_logs, profile_gateway, read_only_data_dir, read_small_file, RequestScope,
};

const RECENT_LOG_LIMIT: i64 = 256;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const QUERY_BUDGET: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OutputSpeedSelection {
    pub model: Option<String>,
    pub service_tier: Option<String>,
}

impl OutputSpeedSelection {
    pub(super) fn normalized(&self) -> Self {
        Self {
            model: self.model.as_deref().and_then(nonempty),
            service_tier: self
                .service_tier
                .as_deref()
                .and_then(normalize_service_tier),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadOutputSpeed {
    pub thread_id: String,
    pub binding: Option<String>,
    /// Session aggregate; deliberately independent from the currently selected mode.
    pub tokens_per_second: f64,
    pub samples: u32,
    pub observed_at_ms: i64,
    /// Latest eligible completed request for the exact selected model and mode.
    pub latest_tokens_per_second: Option<f64>,
    pub latest_input_tokens: Option<i64>,
    pub latest_cached_tokens: Option<i64>,
    pub latest_cache_hit_rate: Option<f64>,
    /// Input-weighted ratio over successful requests with reliable counters.
    pub avg_cache_hit_rate: Option<f64>,
    pub total_input_tokens: Option<i64>,
    pub total_cached_tokens: Option<i64>,
    pub latest_observed_at_ms: Option<i64>,
    pub request_id: Option<String>,
    pub model: Option<String>,
    pub requested_service_tier: Option<String>,
    pub response_service_tier: Option<String>,
    pub selection: OutputSpeedSelection,
}

pub(super) struct SpeedRead {
    pub binding: Option<String>,
    pub selection: OutputSpeedSelection,
    pub result: Result<Option<ThreadOutputSpeed>, String>,
    pub transient_failure: bool,
}

impl SpeedRead {
    pub(super) fn failed(selection: OutputSpeedSelection, error: String) -> Self {
        Self {
            binding: None,
            selection,
            result: Err(error),
            transient_failure: false,
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(super) fn normalize_service_tier(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fast" | "priority" => Some("priority".into()),
        "standard" | "default" => Some("default".into()),
        "auto" => Some("auto".into()),
        "flex" => Some("flex".into()),
        "ultrafast" => Some("ultrafast".into()),
        _ => None,
    }
}

/// A missing tier is unknown, not evidence of normal/fast operation. Native
/// per-chat selection, when available, is supplied separately by the caller.
pub(super) fn read_profile_selection(profile: &Path) -> Result<OutputSpeedSelection, String> {
    let Some(content) = read_small_file(&profile.join("config.toml"))? else {
        return Ok(OutputSpeedSelection::default());
    };
    let document = content
        .parse::<toml_edit::Document>()
        .map_err(|error| format!("output-speed selection unavailable: {error}"))?;
    let selected_profile = document
        .get("profile")
        .and_then(toml_edit::Item::as_str)
        .and_then(|name| document.get("profiles")?.get(name));
    let selected = |key: &str| {
        selected_profile
            .and_then(|profile| profile.get(key))
            .or_else(|| document.get(key))
            .and_then(toml_edit::Item::as_str)
    };
    Ok(OutputSpeedSelection {
        model: selected("model").and_then(nonempty),
        service_tier: selected("service_tier").and_then(normalize_service_tier),
    })
}

/// Fingerprint local binding inputs, never exposing secret text in a payload.
/// DB replacement invalidates it, normal appends/WAL changes do not.
pub(super) fn binding_snapshot(profile: &Path) -> Result<Option<String>, String> {
    let Some((base_url, bearer, scope)) = profile_gateway(profile)? else {
        return Ok(None);
    };
    let data_dir = read_only_data_dir()?;
    let Some(service) = read_small_file(&data_dir.join("codex_local_access.json"))? else {
        return Ok(None);
    };
    let service: serde_json::Value = serde_json::from_str(&service)
        .map_err(|error| format!("output-speed service metadata unavailable: {error}"))?;
    let Some(service_binding) = service_binding_fields(&service, &bearer) else {
        return Ok(None);
    };
    let path = data_dir.join("codex_local_access_logs.sqlite");
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "output-speed database identity unavailable: {error}"
            ))
        }
    };
    let mut hash = Sha256::new();
    let mut field = |value: &[u8]| {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    };
    field(profile.to_string_lossy().as_bytes());
    field(base_url.as_bytes());
    field(bearer.as_bytes());
    field(scope.instance_id.as_deref().unwrap_or("").as_bytes());
    field(scope.installation_id.as_deref().unwrap_or("").as_bytes());
    field(service_binding.to_string().as_bytes());
    field(path.to_string_lossy().as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        field(&metadata.dev().to_le_bytes());
        field(&metadata.ino().to_le_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_READ_ATTRIBUTES,
        };
        // NTFS can preserve creation time when a file is replaced at the same
        // path. Include its real identity, just as dev/ino do on Unix.
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .open(&path)
            .map_err(|error| format!("output-speed database identity unavailable: {error}"))?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // The owned file keeps this handle valid throughout the query.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|error| format!("output-speed database identity unavailable: {error}"))?;
        field(&info.dwVolumeSerialNumber.to_le_bytes());
        field(&info.nFileIndexHigh.to_le_bytes());
        field(&info.nFileIndexLow.to_le_bytes());
    }
    field(format!("{:?}", metadata.created().ok()).as_bytes());
    Ok(Some(format!("{:x}", hash.finalize())))
}

fn selected_fields(value: &serde_json::Value, names: &[&str]) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    for name in names {
        if let Some(value) = value.get(*name) {
            output.insert((*name).into(), value.clone());
        }
    }
    serde_json::Value::Object(output)
}

pub(super) fn service_binding_fields(
    service: &serde_json::Value,
    bearer: &str,
) -> Option<serde_json::Value> {
    let mut selected = selected_fields(
        service,
        &[
            "port",
            "enabled",
            "accessScope",
            "gatewayMode",
            "boundOauthAccountId",
            "accountIds",
            "pausedAccountIds",
            "officialAccountBindings",
            "excludedModels",
            "restrictFreeAccounts",
            "modelAliases",
            "customRoutingRules",
            "accountModelRules",
        ],
    );
    let keys = service.get("apiKeys").and_then(serde_json::Value::as_array);
    let key = if let Some(keys) = keys.filter(|keys| !keys.is_empty()) {
        let key = keys
            .iter()
            .find(|key| key.get("key").and_then(serde_json::Value::as_str) == Some(bearer))?;
        selected_fields(
            key,
            &[
                "id",
                "key",
                "enabled",
                "providerGateway",
                "modelRouting",
                "inheritAccountPool",
                "accountIds",
                "priorityAccountIds",
                "modelPrefix",
                "allowedModels",
                "excludedModels",
                "tokenLimit",
            ],
        )
    } else {
        if service.get("apiKey").and_then(serde_json::Value::as_str) != Some(bearer) {
            return None;
        }
        selected_fields(service, &["apiKey"])
    };
    selected.as_object_mut()?.insert("currentKey".into(), key);
    Some(selected)
}

pub(super) fn read_output_speed(
    profile: &Path,
    thread_id: &str,
) -> Result<Option<ThreadOutputSpeed>, String> {
    read_output_speed_with_selection(profile, thread_id, &OutputSpeedSelection::default()).result
}

pub(super) fn read_output_speed_with_selection(
    profile: &Path,
    thread_id: &str,
    selection: &OutputSpeedSelection,
) -> SpeedRead {
    let selection = selection.normalized();
    let mut verified_binding = None;
    let mut operation = || -> Result<Option<ThreadOutputSpeed>, String> {
        let thread = uuid::Uuid::parse_str(thread_id)
            .map_err(|_| "invalid output-speed thread ID".to_string())?
            .hyphenated()
            .to_string();
        let Some(binding) = binding_snapshot(profile)? else {
            return Ok(None);
        };
        let Some((conn, scope)) = open_gateway_logs(profile)? else {
            return Ok(None);
        };
        // A local installation_id file is not proof of an outgoing header.
        if scope.instance_id.is_none()
            || !super::dialog_data::matches_local_service(profile, &conn, &scope)?
        {
            return Ok(None);
        }
        let mut result = read_with_budget(&conn, &thread, &scope, &selection);
        // Check both success and failure; an old measurement cannot survive a
        // key/account/service change during the query.
        if binding_snapshot(profile)?.as_deref() != Some(binding.as_str()) {
            return Ok(None);
        }
        if let Ok(Some(speed)) = &mut result {
            speed.binding = Some(binding.clone())
        }
        verified_binding = Some(binding);
        result
    };
    let result = operation();
    let transient_failure = result
        .as_ref()
        .err()
        .is_some_and(|error| transient_query_error(error));
    SpeedRead {
        binding: verified_binding,
        selection,
        result,
        transient_failure,
    }
}

fn transient_query_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("database is locked")
        || error.contains("database is busy")
        || error.contains("database table is locked")
        || error.contains("database busy")
        || error.contains("interrupted")
        || error == "output-speed query budget exceeded"
}

fn read_with_budget(
    conn: &Connection,
    thread: &str,
    scope: &RequestScope,
    selection: &OutputSpeedSelection,
) -> Result<Option<ThreadOutputSpeed>, String> {
    let started = Instant::now();
    let interrupt = conn.get_interrupt_handle();
    let (finished, completion) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|threads| {
        let _timer = std::thread::Builder::new()
            .name("codex-output-speed-budget".into())
            .spawn_scoped(threads, move || {
                if matches!(
                    completion.recv_timeout(QUERY_BUDGET),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    interrupt.interrupt()
                }
            })
            .map_err(|error| format!("output-speed query timer unavailable: {error}"))?;
        let result = read_samples_with_selection(conn, thread, scope, selection, started);
        let _ = finished.send(());
        result
    })
}

fn missing_schema(error: &rusqlite::Error) -> bool {
    let error = error.to_string();
    error.contains("no such table")
        || error.contains("no such column")
        || error.contains("no such index")
}

fn summary_query() -> &'static str {
    r#"SELECT s.timestamp,s.client_instance_id,
            json_extract(s.data_json,'$.success'),
            COALESCE(json_extract(s.data_json,'$.errorCategory'),''),
            json_extract(s.data_json,'$.outputTokens'),
            json_extract(s.data_json,'$.outputGenerationMs'),
            s.client_identity_captured,s.client_thread_id,
            s.client_identity_metadata_captured,s.client_installation_id,
            s.request_id,json_extract(s.data_json,'$.model'),
            json_extract(s.data_json,'$.requestedModel'),
            json_extract(s.data_json,'$.requestedServiceTier'),
            json_extract(s.data_json,'$.responseServiceTier'),
            json_extract(s.data_json,'$.inputTokens'),
            json_extract(s.data_json,'$.cachedTokens')
        FROM request_display_summaries AS s INDEXED BY idx_request_display_chat
        WHERE s.api_key_id=?1 AND s.client_instance_id=?2 AND s.client_thread_id=?3
            AND s.dispatched=1 AND length(CAST(s.data_json AS BLOB))<=16384
            AND json_valid(s.data_json)
        ORDER BY s.timestamp DESC,s.log_id DESC LIMIT ?4"#
}

fn legacy_query(conn: &Connection) -> Result<String, String> {
    let mut statement = conn
        .prepare("PRAGMA table_info(request_logs)")
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    let optional = |name: &str| {
        if columns.contains(name) {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        }
    };
    Ok(format!(
        r#"SELECT recent.timestamp,recent.client_instance_id,recent.success,
            recent.error_category,recent.output_tokens,recent.output_generation_ms,
            d.client_identity_captured,d.client_thread_id,
            d.client_identity_metadata_captured,d.client_installation_id,
            recent.request_id,recent.model_id,recent.requested_model,
            recent.requested_service_tier,recent.response_service_tier,
            recent.input_tokens,recent.cached_tokens
        FROM (SELECT id,timestamp,client_instance_id,success,error_category,
                     output_tokens,output_generation_ms,request_id,{},{},{},{},{},{}
              FROM request_logs INDEXED BY idx_codex_local_access_logs_api_key
              WHERE api_key_id=?1 ORDER BY timestamp DESC,id DESC LIMIT ?2) AS recent
        LEFT JOIN request_diagnostics AS d INDEXED BY sqlite_autoindex_request_diagnostics_1
            ON d.request_id=recent.request_id
        ORDER BY recent.timestamp DESC,recent.id DESC"#,
        optional("model_id"),
        optional("requested_model"),
        optional("requested_service_tier"),
        optional("response_service_tier"),
        optional("input_tokens"),
        optional("cached_tokens")
    ))
}

#[cfg(test)]
fn read_samples(
    conn: &Connection,
    thread_id: &str,
    scope: &RequestScope,
    started: Instant,
) -> Result<Option<ThreadOutputSpeed>, String> {
    read_samples_with_selection(
        conn,
        thread_id,
        scope,
        &OutputSpeedSelection::default(),
        started,
    )
}

fn read_samples_with_selection(
    conn: &Connection,
    thread_id: &str,
    scope: &RequestScope,
    selection: &OutputSpeedSelection,
    started: Instant,
) -> Result<Option<ThreadOutputSpeed>, String> {
    // New summaries have a direct thread index. Legacy reads remain bounded
    // and never migrate history, load request bodies or parse attempts JSON.
    let (mut statement, legacy) = match conn.prepare(summary_query()) {
        Ok(statement) => (statement, false),
        Err(error) if missing_schema(&error) => match conn.prepare(&legacy_query(conn)?) {
            Ok(statement) => (statement, true),
            Err(error) if missing_schema(&error) => return Ok(None),
            Err(error) => return Err(format!("output-speed query unavailable: {error}")),
        },
        Err(error) => return Err(format!("output-speed query unavailable: {error}")),
    };
    let mut rows = if legacy {
        statement.query(rusqlite::params![scope.key_id, RECENT_LOG_LIMIT])
    } else {
        statement.query(rusqlite::params![
            scope.key_id,
            scope.instance_id,
            thread_id,
            RECENT_LOG_LIMIT
        ])
    }
    .map_err(|error| format!("output-speed query failed: {error}"))?;
    let selection = selection.normalized();
    let mut tokens: i64 = 0;
    let mut generation_ms: i64 = 0;
    let mut samples: u32 = 0;
    let mut observed_at_ms: i64 = 0;
    let mut latest = None;
    let mut cache_totals = Some((0_i64, 0_i64));
    let mut cache_samples = 0;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        if started.elapsed() > QUERY_BUDGET {
            return Err("output-speed query budget exceeded".into());
        }
        let instance: String = row.get(1).map_err(|error| error.to_string())?;
        let captured: Option<i64> = row.get(6).map_err(|error| error.to_string())?;
        let thread: Option<String> = row.get(7).map_err(|error| error.to_string())?;
        let metadata: Option<i64> = row.get(8).map_err(|error| error.to_string())?;
        let installation: Option<String> = row.get(9).map_err(|error| error.to_string())?;
        if captured != Some(1)
            || metadata != Some(1)
            || Some(&instance) != scope.instance_id.as_ref()
            || scope
                .installation_id
                .as_ref()
                .is_some_and(|expected| installation.as_ref() != Some(expected))
            || thread
                .as_deref()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(|id| id.hyphenated().to_string())
                .as_deref()
                != Some(thread_id)
        {
            continue;
        }
        let success: i64 = row.get(2).map_err(|error| error.to_string())?;
        let error_category: String = row.get(3).map_err(|error| error.to_string())?;
        let output_tokens: Option<i64> = row.get(4).map_err(|error| error.to_string())?;
        let output_ms: Option<i64> = row.get(5).map_err(|error| error.to_string())?;
        if success != 1 || !error_category.is_empty() {
            continue;
        }
        // Match src/utils/codexOutputSpeed.ts. No latency-based inference.
        let rate = (output_tokens.is_some_and(|tokens| (100..=MAX_SAFE_INTEGER).contains(&tokens))
            && output_ms.is_some_and(|ms| (100..=MAX_SAFE_INTEGER).contains(&ms)))
        .then(|| {
            output_tokens.unwrap_or_default() as f64
                / (output_ms.unwrap_or_default() as f64 / 1000.0)
        });
        let input_tokens: Option<i64> = row.get(15).map_err(|error| error.to_string())?;
        let cached_tokens: Option<i64> = row.get(16).map_err(|error| error.to_string())?;
        let input_tokens = input_tokens.filter(|tokens| (1..=MAX_SAFE_INTEGER).contains(tokens));
        let cached_tokens = cached_tokens
            .filter(|cached| input_tokens.is_some_and(|input| (0..=input).contains(cached)));
        let cache_rate = input_tokens
            .zip(cached_tokens)
            .map(|(input, cached)| cached as f64 / input as f64);
        if rate.is_none() && cache_rate.is_none() {
            continue;
        }
        if rate.is_some() {
            tokens += output_tokens.unwrap_or_default();
            generation_ms += output_ms.unwrap_or_default();
            if tokens > MAX_SAFE_INTEGER || generation_ms > MAX_SAFE_INTEGER {
                return Ok(None);
            }
            samples += 1;
        }
        if let Some((input, cached)) = input_tokens.zip(cached_tokens) {
            cache_samples += 1;
            cache_totals = cache_totals.and_then(|(all_input, all_cached)| {
                let next_input = all_input.checked_add(input)?;
                let next_cached = all_cached.checked_add(cached)?;
                (next_input <= MAX_SAFE_INTEGER && next_cached <= MAX_SAFE_INTEGER)
                    .then_some((next_input, next_cached))
            });
        }
        let timestamp: i64 = row.get(0).map_err(|error| error.to_string())?;
        observed_at_ms = observed_at_ms.max(timestamp);
        if latest.is_none() && selection.model.is_some() {
            let model: Option<String> = row.get(11).map_err(|error| error.to_string())?;
            let requested_model: Option<String> = row.get(12).map_err(|error| error.to_string())?;
            let requested_tier: Option<String> = row.get(13).map_err(|error| error.to_string())?;
            let response_tier: Option<String> = row.get(14).map_err(|error| error.to_string())?;
            let model = requested_model
                .as_deref()
                .and_then(nonempty)
                .or_else(|| model.as_deref().and_then(nonempty));
            let raw_tier = requested_tier.as_deref().and_then(nonempty);
            let tier_matches = match (&selection.service_tier, raw_tier) {
                (None, None) => true, // both requests and the selection leave it unspecified
                (Some(expected), None) if expected == "default" => {
                    // The native client can omit standard mode on the wire.
                    // Confirm it from the response; never reinterpret an explicit
                    // priority request that the upstream downgraded to default.
                    response_tier
                        .as_deref()
                        .and_then(normalize_service_tier)
                        .as_deref()
                        == Some("default")
                }
                (Some(expected), Some(recorded)) => {
                    normalize_service_tier(&recorded).as_ref() == Some(expected)
                }
                _ => false,
            };
            if model == selection.model && tier_matches {
                latest = Some((
                    rate,
                    timestamp,
                    row.get::<_, String>(10)
                        .map_err(|error| error.to_string())?,
                    model,
                    requested_tier,
                    response_tier,
                    input_tokens,
                    cached_tokens,
                    cache_rate,
                ));
            }
        }
    }
    if started.elapsed() > QUERY_BUDGET {
        return Err("output-speed query budget exceeded".into());
    }
    Ok(
        (samples > 0 || cache_samples > 0 || latest.is_some()).then(|| ThreadOutputSpeed {
            thread_id: thread_id.to_string(),
            binding: None,
            tokens_per_second: if generation_ms > 0 {
                tokens as f64 / (generation_ms as f64 / 1000.0)
            } else {
                0.0
            },
            samples,
            observed_at_ms,
            latest_tokens_per_second: latest.as_ref().and_then(|sample| sample.0),
            latest_input_tokens: latest.as_ref().and_then(|sample| sample.6),
            latest_cached_tokens: latest.as_ref().and_then(|sample| sample.7),
            latest_cache_hit_rate: latest.as_ref().and_then(|sample| sample.8),
            avg_cache_hit_rate: cache_totals
                .filter(|(input, _)| *input > 0)
                .map(|(input, cached)| cached as f64 / input as f64),
            total_input_tokens: cache_totals
                .filter(|(input, _)| *input > 0)
                .map(|totals| totals.0),
            total_cached_tokens: cache_totals
                .filter(|(input, _)| *input > 0)
                .map(|totals| totals.1),
            latest_observed_at_ms: latest.as_ref().map(|sample| sample.1),
            request_id: latest.as_ref().map(|sample| sample.2.clone()),
            model: latest.as_ref().and_then(|sample| sample.3.clone()),
            requested_service_tier: latest.as_ref().and_then(|sample| sample.4.clone()),
            response_service_tier: latest.and_then(|sample| sample.5),
            selection,
        }),
    )
}

#[cfg(test)]
#[path = "codex_app_injection_output_speed_tests.rs"]
mod tests;
