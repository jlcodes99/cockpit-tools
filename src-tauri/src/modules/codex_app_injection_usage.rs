//! Bounded, read-only usage snapshots for the injected Codex status line.
//!
//! Callers must run this synchronous reader on a blocking worker. It never reads
//! conversation content into the returned snapshot, or scans another profile.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{Datelike, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

const TAIL_INITIAL_BYTES: u64 = 512 * 1024;
const TAIL_MAX_BYTES: u64 = 8 * 1024 * 1024;
const EVENT_LIMIT: usize = 24;
const MAX_SCAN_ENTRIES: usize = 2048;
const MAX_CACHE_ENTRIES: usize = 64;
const READ_BUDGET: Duration = Duration::from_millis(250);
const SCAN_BUDGET: Duration = Duration::from_millis(60);
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InjectionUsageSnapshot {
    pub thread_id: String,
    pub events: Vec<Value>,
    // A new task may exist before its first token_count. Keeping its ID prevents
    // the renderer from presenting the preceding task's cached metrics.
    pub current_turn_id: Option<String>,
}

#[derive(Clone)]
struct CachedUsage {
    path: Option<PathBuf>,
    stamp: Option<(u64, Option<SystemTime>)>,
    snapshot: InjectionUsageSnapshot,
    checked_at: Instant,
}

type CacheKey = (PathBuf, String);
static CACHE: OnceLock<Mutex<HashMap<CacheKey, CachedUsage>>> = OnceLock::new();

fn cached_usage(key: &CacheKey) -> Option<CachedUsage> {
    CACHE
        .get_or_init(Default::default)
        .try_lock()
        .ok()?
        .get(key)
        .cloned()
}

fn remember_usage(key: CacheKey, cached: CachedUsage) {
    let Ok(mut cache) = CACHE.get_or_init(Default::default).try_lock() else {
        return;
    };
    if cache.len() >= MAX_CACHE_ENTRIES && !cache.contains_key(&key) {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, item)| item.checked_at)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, cached);
}

/// Read only this profile's rollout, using the state DB's primary-key lookup
/// first. Missing new sessions are normal and return an empty snapshot.
pub(super) fn read_usage_snapshot(
    profile_dir: &Path,
    thread_id: &str,
) -> Result<InjectionUsageSnapshot, String> {
    let started = Instant::now();
    let thread_id = normalize_thread_id(thread_id)?;
    let empty = InjectionUsageSnapshot {
        thread_id: thread_id.clone(),
        ..Default::default()
    };
    let profile = match profile_dir.canonicalize() {
        Ok(profile) => profile,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(empty),
        Err(error) => return Err(format!("usage profile unavailable: {error}")),
    };
    let key = (profile.clone(), thread_id.clone());
    let cached = cached_usage(&key);
    if let Some(cached) = &cached {
        if cached.path.is_none() && cached.checked_at.elapsed() < NEGATIVE_CACHE_TTL {
            return Ok(cached.snapshot.clone());
        }
    }
    let roots = session_roots(&profile)?;
    let cached_path = cached
        .as_ref()
        .and_then(|entry| entry.path.as_ref())
        .and_then(|path| validate_rollout_path(&profile, &roots, path, &thread_id));
    // Resuming a thread can rotate its rollout while the old file still exists.
    // Consult the indexed path before falling back to the cached location.
    let path = locate_rollout(&profile, &roots, &thread_id, cached_path, started)?;
    let Some(path) = path else {
        remember_usage(
            key,
            CachedUsage {
                path: None,
                stamp: None,
                snapshot: empty.clone(),
                checked_at: Instant::now(),
            },
        );
        return Ok(empty);
    };
    check_budget(started)?;
    let mut file =
        File::open(&path).map_err(|error| format!("usage rollout unavailable: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("usage metadata unavailable: {error}"))?;
    let stamp = (metadata.len(), metadata.modified().ok());
    if let Some(cached) = cached {
        if cached.path.as_ref() == Some(&path) && cached.stamp.as_ref() == Some(&stamp) {
            return Ok(cached.snapshot);
        }
    }
    let (events, current_turn_id) = read_usage_tail(&mut file, metadata.len(), started)?;
    let snapshot = InjectionUsageSnapshot {
        thread_id,
        events,
        current_turn_id,
    };
    remember_usage(
        key,
        CachedUsage {
            path: Some(path),
            stamp: Some(stamp),
            snapshot: snapshot.clone(),
            checked_at: Instant::now(),
        },
    );
    Ok(snapshot)
}

fn normalize_thread_id(raw: &str) -> Result<String, String> {
    let id = raw.strip_prefix("local:").unwrap_or(raw);
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "invalid usage thread ID".to_string())?;
    let normalized = parsed.hyphenated().to_string();
    if !id.eq_ignore_ascii_case(&normalized) {
        return Err("invalid usage thread ID".to_string());
    }
    Ok(normalized)
}

fn session_roots(profile: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    for name in ["sessions", "archived_sessions"] {
        let root = match profile.join(name).canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("usage sessions unavailable: {error}")),
        };
        if root.starts_with(profile) && root.is_dir() {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn rollout_filename_matches(filename: &str, thread_id: &str) -> bool {
    let Some(stem) = filename
        .strip_prefix("rollout-")
        .and_then(|name| name.strip_suffix(".jsonl"))
    else {
        return false;
    };
    // Resumed rollouts append a UUID after the original thread ID. Match the
    // original ID only: the suffix must never identify a different thread.
    let base = match stem.rsplit_once('_') {
        Some((base, suffix)) => {
            let Ok(id) = uuid::Uuid::parse_str(suffix) else {
                return false;
            };
            if !id.hyphenated().to_string().eq_ignore_ascii_case(suffix) {
                return false;
            }
            base
        }
        None => stem,
    };
    base.ends_with(&format!("-{thread_id}"))
}

fn validate_rollout_path(
    profile: &Path,
    roots: &[PathBuf],
    candidate: &Path,
    thread_id: &str,
) -> Option<PathBuf> {
    if candidate
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        profile.join(candidate)
    };
    let filename = candidate.file_name()?.to_str()?;
    if !rollout_filename_matches(filename, thread_id) {
        return None;
    }
    let canonical = candidate.canonicalize().ok()?;
    // An in-profile symlink must not map one thread's filename to another's.
    if !rollout_filename_matches(canonical.file_name()?.to_str()?, thread_id) {
        return None;
    }
    if !canonical.starts_with(profile) || !roots.iter().any(|root| canonical.starts_with(root)) {
        return None;
    }
    canonical.is_file().then_some(canonical)
}

fn check_budget(started: Instant) -> Result<(), String> {
    if started.elapsed() > READ_BUDGET {
        Err("usage read budget exceeded".to_string())
    } else {
        Ok(())
    }
}

fn locate_rollout(
    profile: &Path,
    roots: &[PathBuf],
    thread_id: &str,
    cached_path: Option<PathBuf>,
    started: Instant,
) -> Result<Option<PathBuf>, String> {
    for db_path in [
        profile.join("state_5.sqlite"),
        profile.join("sqlite/state_5.sqlite"),
    ] {
        check_budget(started)?;
        // A profile DB may itself be a symlink. Do not open a different
        // profile's database even when its returned rollout would be rejected.
        let Ok(db_path) = db_path.canonicalize() else {
            continue;
        };
        if !db_path.starts_with(profile) {
            continue;
        }
        let Ok(db) = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        if db.busy_timeout(Duration::from_millis(50)).is_err() {
            continue;
        }
        // A malformed/foreign schema must not turn this into a table scan.
        let indexed_id = db
            .prepare("PRAGMA table_info(threads)")
            .and_then(|mut statement| {
                let rows = statement.query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
                })?;
                for row in rows {
                    let (name, primary_key) = row?;
                    if name == "id" && primary_key == 1 {
                        return Ok(true);
                    }
                }
                Ok(false)
            })
            .unwrap_or(false);
        if !indexed_id {
            continue;
        }
        let found = db
            .query_row(
                "SELECT rollout_path FROM threads WHERE id = ?1 LIMIT 1",
                [thread_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional();
        if let Ok(Some(Some(candidate))) = found {
            if let Some(path) =
                validate_rollout_path(profile, roots, Path::new(&candidate), thread_id)
            {
                return Ok(Some(path));
            }
        }
    }
    if cached_path.is_some() {
        return Ok(cached_path);
    }
    bounded_rollout_search(profile, roots, thread_id, started)
}

fn bounded_rollout_search(
    profile: &Path,
    roots: &[PathBuf],
    thread_id: &str,
    started: Instant,
) -> Result<Option<PathBuf>, String> {
    let scan_started = Instant::now();
    let mut directories = VecDeque::new();
    // Official UUIDv7 IDs encode creation time: probe that date directly before
    // the bounded fallback, so large unrelated archives do not hide new files.
    if let Ok(id) = uuid::Uuid::parse_str(thread_id) {
        if id.get_version_num() == 7 {
            let mut bytes = [0u8; 8];
            bytes[2..].copy_from_slice(&id.as_bytes()[..6]);
            if let Some(date) =
                chrono::DateTime::<Utc>::from_timestamp_millis(i64::from_be_bytes(bytes))
            {
                for root in roots {
                    directories.push_back((
                        root.join(format!(
                            "{:04}/{:02}/{:02}",
                            date.year(),
                            date.month(),
                            date.day()
                        )),
                        3,
                    ));
                }
            }
        }
    }
    directories.extend(roots.iter().cloned().map(|root| (root, 0)));
    let mut entries_seen = 0;
    while let Some((directory, depth)) = directories.pop_front() {
        check_budget(started)?;
        if scan_started.elapsed() > SCAN_BUDGET || entries_seen >= MAX_SCAN_ENTRIES {
            return Err("usage lookup budget exceeded".to_string());
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("usage lookup failed: {error}")),
        };
        for entry in entries {
            entries_seen += 1;
            if entries_seen > MAX_SCAN_ENTRIES || scan_started.elapsed() > SCAN_BUDGET {
                return Err("usage lookup budget exceeded".to_string());
            }
            let entry = entry.map_err(|error| format!("usage lookup entry failed: {error}"))?;
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("usage lookup entry failed: {error}")),
            };
            // Never descend into symlinks, and stop after sessions/yyyy/mm/dd.
            if kind.is_dir() && depth < 3 {
                directories.push_back((entry.path(), depth + 1));
            } else if kind.is_file() {
                if let Some(path) = validate_rollout_path(profile, roots, &entry.path(), thread_id)
                {
                    return Ok(Some(path));
                }
            }
        }
    }
    Ok(None)
}

fn read_usage_tail(
    file: &mut File,
    length: u64,
    started: Instant,
) -> Result<(Vec<Value>, Option<String>), String> {
    let mut window = TAIL_INITIAL_BYTES;
    loop {
        check_budget(started)?;
        let start = length.saturating_sub(window);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| format!("usage seek failed: {error}"))?;
        let mut buffer = Vec::with_capacity((length - start) as usize);
        // Keep the initial metadata length as a hard cap, even if the active
        // rollout grows while this worker is reading it.
        file.take(length - start)
            .read_to_end(&mut buffer)
            .map_err(|error| format!("usage tail read failed: {error}"))?;
        let snapshot = parse_usage_window(&buffer, start > 0, started)?;
        if snapshot.0.len() >= 2 || start == 0 || window >= TAIL_MAX_BYTES {
            return Ok(snapshot);
        }
        window = (window * 4).min(TAIL_MAX_BYTES);
    }
}

fn parse_usage_window(
    bytes: &[u8],
    starts_midline: bool,
    started: Instant,
) -> Result<(Vec<Value>, Option<String>), String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split_terminator('\n');
    if starts_midline {
        lines.next();
    }
    let complete_end = bytes.last() == Some(&b'\n');
    let mut lines = lines.peekable();
    let mut events = VecDeque::new();
    let mut current_turn_id = None;
    let mut previous_signature = None;
    while let Some(line) = lines.next() {
        check_budget(started)?;
        if !complete_end && lines.peek().is_none() {
            break;
        }
        if line.len() > 512 * 1024
            || (!line.contains("\"token_count\"") && !line.contains("\"task_started\""))
        {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) == Some("task_started") {
            current_turn_id = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .or_else(|| value.get("timestamp").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            previous_signature = None;
            continue;
        }
        let Some((event, signature)) = usage_event(&value, current_turn_id.as_deref()) else {
            continue;
        };
        if previous_signature.as_ref() == Some(&signature) {
            continue;
        }
        previous_signature = Some(signature);
        events.push_back(event);
        if events.len() > EVENT_LIMIT {
            events.pop_front();
        }
    }
    Ok((events.into_iter().collect(), current_turn_id))
}

fn usage_event(value: &Value, turn_id: Option<&str>) -> Option<(Value, Value)> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let info = payload.get("info")?;
    let last = info.get("last_token_usage")?;
    let field = |source: Option<&Value>, key: &str| {
        source
            .and_then(|source| source.get(key))
            .and_then(Value::as_u64)
    };
    let input = field(Some(last), "input_tokens");
    let output = field(Some(last), "output_tokens");
    let total = field(Some(last), "total_tokens");
    if input.unwrap_or(0) == 0 && output.unwrap_or(0) == 0 && total.unwrap_or(0) == 0 {
        return None;
    }
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let aggregate = info.get("total_token_usage");
    let has_aggregate_counters = [
        "input_tokens",
        "cached_input_tokens",
        "output_tokens",
        "total_tokens",
    ]
    .iter()
    .any(|key| field(aggregate, key).is_some());
    let event = json!({
        "t": timestamp,
        "input_tokens": input,
        "cached_input_tokens": field(Some(last), "cached_input_tokens"),
        "output_tokens": output,
        "reasoning_output_tokens": field(Some(last), "reasoning_output_tokens"),
        "total_tokens": total,
        "context_window": field(Some(info), "model_context_window"),
        "turn_output_tokens": field(aggregate, "output_tokens"),
        "turn_id": turn_id,
    });
    // Codex can write the same accounting snapshot more than once. Time alone
    // must not make that a second model call. Without cumulative counters,
    // preserve timestamp to avoid conflating equal-sized legitimate requests.
    let signature = json!([
        turn_id,
        input,
        field(Some(last), "cached_input_tokens"),
        output,
        total,
        field(Some(last), "reasoning_output_tokens"),
        field(aggregate, "input_tokens"),
        field(aggregate, "cached_input_tokens"),
        field(aggregate, "output_tokens"),
        field(aggregate, "total_tokens"),
        if has_aggregate_counters {
            ""
        } else {
            timestamp
        },
    ]);
    Some((event, signature))
}

#[cfg(test)]
#[path = "codex_app_injection_usage_tests.rs"]
mod tests;
