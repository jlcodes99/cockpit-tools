//! Read the latest request's actual account without changing log schemas or
//! loading request bodies. All I/O must run on the caller's blocking worker.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

const RECENT_LOG_LIMIT: i64 = 256;
const CONFIG_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecentUsedAccount {
    pub account_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub last_used_at_ms: i64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredKeys {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    api_keys: Vec<StoredKey>,
    #[serde(default)]
    gateway_mode: String,
}

#[derive(Deserialize)]
struct StoredKey {
    id: String,
    key: String,
}

#[derive(Default)]
pub(super) struct RequestScope {
    pub key_id: String,
    pub instance_id: Option<String>,
    pub installation_id: Option<String>,
}

#[derive(Debug)]
struct ActualAccount {
    account_id: String,
    email: Option<String>,
    last_used_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedAttempt {
    #[serde(default)]
    sequence: u32,
    #[serde(default)]
    account_id: String,
    account_email: Option<String>,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    started_at_ms: i64,
    status: Option<u16>,
    failure_phase: Option<String>,
}

/// Account identity comes only from a real logged request. Local account
/// metadata is consulted afterwards solely for its display name and raw plan.
pub(super) fn read_recent_account(
    profile_dir: &Path,
    thread_id: &str,
) -> Result<Option<RecentUsedAccount>, String> {
    let thread = uuid::Uuid::parse_str(thread_id)
        .map_err(|_| "invalid recent-account thread ID".to_string())?
        .hyphenated()
        .to_string();
    let Some((conn, scope)) = open_gateway_logs(profile_dir)? else {
        return Ok(None);
    };
    let Some(actual) = read_actual_account(&conn, &thread, &scope)? else {
        return Ok(None);
    };
    let mut display_name = actual
        .email
        .clone()
        .unwrap_or_else(|| actual.account_id.clone());
    let mut email = actual.email;
    let mut plan = None;
    // Stored logs are untrusted too; never turn an account ID into a path until
    // it has passed the same narrow filename-safe boundary as official IDs.
    if safe_account_id(&actual.account_id) {
        if let Some(account) = crate::modules::codex_account::load_account(&actual.account_id) {
            let name = account.display_name("");
            if !name.trim().is_empty() {
                display_name = name;
            }
            if email.is_none() {
                email = nonempty(Some(account.email));
            }
            plan = account.plan_type.or(account.auth_file_plan_type);
        }
    }
    Ok(Some(RecentUsedAccount {
        account_id: actual.account_id,
        display_name,
        email,
        plan,
        last_used_at_ms: actual.last_used_at_ms,
    }))
}

/// Resolve only the local gateway configured for this exact profile and open
/// its logs without migrations, schema writes or request-body reads.
pub(super) fn open_gateway_logs(
    profile_dir: &Path,
) -> Result<Option<(Connection, RequestScope)>, String> {
    let Some((base_url, bearer, mut scope)) = profile_gateway(profile_dir)? else {
        return Ok(None);
    };
    let url = reqwest::Url::parse(&base_url).ok();
    if !url.as_ref().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
    }) {
        return Ok(None);
    }
    let data_dir = read_only_data_dir()?;
    let Some(keys) = read_keys(&data_dir.join("codex_local_access.json"))? else {
        return Ok(None);
    };
    let Some(key_id) = key_id_for_bearer(&keys, &bearer) else {
        return Ok(None);
    };
    scope.key_id = key_id;
    let path = data_dir.join("codex_local_access_logs.sqlite");
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("recent-account database unavailable: {error}"))?;
    conn.busy_timeout(Duration::from_millis(50))
        .map_err(|error| error.to_string())?;
    Ok(Some((conn, scope)))
}

pub(super) fn read_only_data_dir() -> Result<std::path::PathBuf, String> {
    // Match account::get_data_dir's overrides without creating a directory on
    // this observational path when the service has never been configured.
    for name in [
        "COCKPIT_TOOLS_TEST_DATA_DIR",
        "COCKPIT_TEST_DATA_DIR",
        "COCKPIT_TOOLS_DATA_DIR",
    ] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Ok(std::path::PathBuf::from(value.trim()));
            }
        }
    }
    #[cfg(test)]
    return Ok(crate::modules::test_support::fallback_data_dir());
    #[cfg(not(test))]
    crate::modules::account::resolve_data_dir()
}

fn read_keys(path: &Path) -> Result<Option<StoredKeys>, String> {
    let Some(content) = read_small_file(path)? else {
        return Ok(None);
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("recent-account key metadata unavailable: {error}"))
}

pub(super) fn read_small_file(path: &Path) -> Result<Option<String>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("recent-account configuration unavailable: {error}")),
    };
    let mut content = String::new();
    file.take(CONFIG_LIMIT + 1)
        .read_to_string(&mut content)
        .map_err(|error| format!("recent-account configuration read failed: {error}"))?;
    if content.len() as u64 > CONFIG_LIMIT {
        return Err("recent-account configuration too large".into());
    }
    Ok(Some(content))
}

fn key_id_for_bearer(keys: &StoredKeys, bearer: &str) -> Option<String> {
    if bearer.is_empty() {
        return None;
    }
    if let Some(key) = keys.api_keys.iter().find(|key| key.key == bearer) {
        return nonempty(Some(key.id.clone()));
    }
    if keys.api_key == bearer && keys.api_keys.is_empty() {
        return Some(
            if keys.gateway_mode == "native" {
                "native:primary"
            } else {
                "legacy"
            }
            .into(),
        );
    }
    None
}

pub(super) fn profile_gateway(
    profile: &Path,
) -> Result<Option<(String, String, RequestScope)>, String> {
    let mut scope = RequestScope::default();
    let Some(content) = read_small_file(&profile.join("config.toml"))? else {
        return Ok(None);
    };
    let document = content
        .parse::<toml_edit::Document>()
        .map_err(|error| format!("recent-account profile metadata unavailable: {error}"))?;
    let selected_profile = document
        .get("profile")
        .and_then(toml_edit::Item::as_str)
        .and_then(|name| document.get("profiles")?.get(name));
    let provider = selected_profile
        .and_then(|profile| profile.get("model_provider"))
        .or_else(|| document.get("model_provider"))
        .and_then(toml_edit::Item::as_str)
        .and_then(|id| document.get("model_providers")?.get(id));
    let Some(provider) = provider else {
        return Ok(None);
    };
    let Some(base_url) = provider.get("base_url").and_then(toml_edit::Item::as_str) else {
        return Ok(None);
    };
    let mut bearer = provider
        .get("experimental_bearer_token")
        .or_else(|| provider.get("api_key"))
        .and_then(toml_edit::Item::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string);
    if bearer.is_none() {
        if let Some(auth) = read_small_file(&profile.join("auth.json"))? {
            let auth: serde_json::Value = serde_json::from_str(&auth)
                .map_err(|error| format!("recent-account auth metadata unavailable: {error}"))?;
            bearer = auth
                .get("OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string);
        }
    }
    let Some(bearer) = bearer else {
        return Ok(None);
    };
    let headers = provider.get("http_headers");
    if let Some(headers) = headers {
        for name in [
            "x-cockpit-instance-id",
            "x-codex-instance-id",
            "x-client-instance-id",
        ] {
            if let Some(value) = headers.get(name).and_then(toml_edit::Item::as_str) {
                scope.instance_id = nonempty(Some(value.to_string()));
                break;
            }
        }
        scope.installation_id = headers
            .get("x-codex-installation-id")
            .and_then(toml_edit::Item::as_str)
            .map(str::to_string)
            .and_then(|value| nonempty(Some(value)));
    }
    Ok(Some((base_url.to_string(), bearer, scope)))
}

fn read_actual_account(
    conn: &Connection,
    thread_id: &str,
    scope: &RequestScope,
) -> Result<Option<ActualAccount>, String> {
    let started = Instant::now();
    // LIMIT is inside the subquery: the existing key/timestamp index bounds
    // rows examined before matching thread identity. Diagnostic joins use its
    // request_id primary key. No payloads_json column is ever selected.
    let query = r#"SELECT recent.account_id, recent.email, recent.timestamp,
            recent.client_instance_id, d.client_identity_captured, d.client_thread_id,
            d.client_identity_metadata_captured, d.client_installation_id,
            CASE WHEN length(CAST(d.attempts_json AS BLOB))<=65536 THEN d.attempts_json END
        FROM (SELECT id,account_id,email,timestamp,client_instance_id,request_id
              FROM request_logs INDEXED BY idx_codex_local_access_logs_api_key
              WHERE api_key_id=?1 ORDER BY timestamp DESC LIMIT ?2) AS recent
        LEFT JOIN request_diagnostics AS d ON d.request_id=recent.request_id
        ORDER BY recent.timestamp DESC,recent.id DESC"#;
    let mut statement = match conn.prepare(query) {
        Ok(statement) => statement,
        Err(error) => {
            // Older logs may not have captured identity/index/schema support.
            // Reading them must not create or migrate anything.
            if error.to_string().contains("no such table")
                || error.to_string().contains("no such column")
                || error.to_string().contains("no such index")
            {
                return Ok(None);
            }
            return Err(format!("recent-account query unavailable: {error}"));
        }
    };
    let mut rows = statement
        .query(rusqlite::params![scope.key_id, RECENT_LOG_LIMIT])
        .map_err(|error| format!("recent-account query failed: {error}"))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("recent-account row failed: {error}"))?
    {
        if started.elapsed() > Duration::from_millis(150) {
            return Err("recent-account query budget exceeded".into());
        }
        let captured: Option<i64> = row.get(4).map_err(|error| error.to_string())?;
        let thread: Option<String> = row.get(5).map_err(|error| error.to_string())?;
        if captured != Some(1)
            || thread
                .as_deref()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(|id| id.hyphenated().to_string())
                .as_deref()
                != Some(thread_id)
        {
            continue;
        }
        if let Some(expected) = &scope.instance_id {
            let actual: String = row.get(3).map_err(|error| error.to_string())?;
            if actual != *expected {
                continue;
            }
        }
        if let Some(expected) = &scope.installation_id {
            let captured: Option<i64> = row.get(6).map_err(|error| error.to_string())?;
            let actual: Option<String> = row.get(7).map_err(|error| error.to_string())?;
            if captured != Some(1) || actual.as_ref() != Some(expected) {
                continue;
            }
        }
        let account_id: String = row.get(0).map_err(|error| error.to_string())?;
        // This is the latest matching request even if routing failed before an
        // account was chosen. Do not resurrect an older successful account.
        if !safe_account_id(&account_id) {
            return Ok(None);
        }
        let attempts: Option<String> = row.get(8).map_err(|error| error.to_string())?;
        let Some(attempt) = actual_attempt(&account_id, attempts.as_deref()) else {
            return Ok(None);
        };
        let email: String = row.get(1).map_err(|error| error.to_string())?;
        let timestamp: i64 = row.get(2).map_err(|error| error.to_string())?;
        return Ok(Some(ActualAccount {
            account_id,
            email: nonempty(Some(email)).or_else(|| nonempty(attempt.account_email)),
            last_used_at_ms: if attempt.started_at_ms > 0 {
                attempt.started_at_ms
            } else {
                timestamp
            },
        }));
    }
    Ok(None)
}

fn actual_attempt(account_id: &str, json: Option<&str>) -> Option<RecordedAttempt> {
    let attempts: Vec<RecordedAttempt> = serde_json::from_str(json?).ok()?;
    let final_attempt = attempts
        .into_iter()
        .filter(|attempt| {
            safe_account_id(&attempt.account_id)
                && !attempt.transport.is_empty()
                && attempt.transport != "preflight"
                && attempt.status.is_some_and(|status| status >= 100)
                && !matches!(
                    attempt.failure_phase.as_deref(),
                    Some("routing" | "identity" | "validation" | "preparation")
                )
        })
        .max_by_key(|attempt| (attempt.sequence, attempt.started_at_ms))?;
    (final_attempt.account_id == account_id).then_some(final_attempt)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    })
}

fn safe_account_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
#[path = "codex_app_injection_recent_account_tests.rs"]
mod tests;
