use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{logger, workbuddy_account};

const TWO_DAYS_MS: i64 = 2 * 24 * 60 * 60 * 1000;
const DEDUP_WINDOW_MS: i64 = 15 * 1000;
const CACHE_TTL_MS: i64 = 5 * 60 * 1000;
const MAX_MESSAGE_CHARS: usize = 500;
const SOURCE_CLIENT_LOG: &str = "client_log";

const RATE_LIMIT_ZH_BYTES: &[u8] = "超出频率限制".as_bytes();
const RATE_LIMIT_EN_BYTES: &[u8] = b"usage exceeds frequency limit";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyLogLimit {
    pub account_id: String,
    pub uid: String,
    pub session_id: String,
    pub model: String,
    pub reset_at: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub hit_count: u32,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone)]
struct RawLogLimitHit {
    account_id: String,
    uid: String,
    session_id: String,
    model: String,
    reset_at: i64,
    occurred_at: i64,
    message: String,
}

struct CachedLogLimits {
    scanned_at_ms: i64,
    items: Vec<WorkbuddyLogLimit>,
}

static LOG_LIMITS_CACHE: LazyLock<Mutex<Option<CachedLogLimits>>> =
    LazyLock::new(|| Mutex::new(None));
static SCAN_EXECUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

static RE_ISO_TZ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))")
        .expect("valid iso regex")
});
static RE_UTC_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(\d{4})[-/](\d{2})[-/](\d{2})[ T](\d{1,2}):(\d{2})(?::(\d{2}))?\s*(?:UTC|GMT)([+-]\d{1,2}(?::?\d{2})?)?",
    )
    .expect("valid utc offset regex")
});
static RE_LOCAL_DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{4})[-/](\d{2})[-/](\d{2})[ T](\d{1,2}):(\d{2})(?::(\d{2}))?")
        .expect("valid local datetime regex")
});

fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_time_to_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub fn default_log_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut roots = vec![home.join(".workbuddy").join("logs")];
    let ai_logs = home.join(".workbuddy-ai").join("logs");
    if ai_logs.is_dir() {
        roots.push(ai_logs);
    }
    roots
}

/// Tauri command: scan WorkBuddy client logs for passive 429 rate-limit entries.
/// Uses a 5-minute in-memory cache unless `force` is `Some(true)`.
#[tauri::command]
pub async fn scan_workbuddy_log_limits(
    force: Option<bool>,
) -> Result<Vec<WorkbuddyLogLimit>, String> {
    let force = force.unwrap_or(false);
    let now_ms = current_time_ms();
    if !force {
        let cache = LOG_LIMITS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.as_ref() {
            if now_ms - cached.scanned_at_ms < CACHE_TTL_MS {
                return Ok(filter_active_limits(&cached.items, now_ms));
            }
        }
    }

    tauri::async_runtime::spawn_blocking(move || scan_and_cache_blocking(force))
        .await
        .map_err(|e| format!("扫描 WorkBuddy 日志限流记录失败: {e}"))?
}

fn scan_and_cache_blocking(force: bool) -> Result<Vec<WorkbuddyLogLimit>, String> {
    let _scan_guard = SCAN_EXECUTION_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now_ms = current_time_ms();
    if !force {
        let cache = LOG_LIMITS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.as_ref() {
            if now_ms - cached.scanned_at_ms < CACHE_TTL_MS {
                return Ok(filter_active_limits(&cached.items, now_ms));
            }
        }
    }

    let roots = default_log_roots();
    let accounts = workbuddy_account::list_accounts();
    let items = scan_workbuddy_log_limits_in_roots(&roots, &accounts, now_ms);

    let mut cache = LOG_LIMITS_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cache = Some(CachedLogLimits {
        scanned_at_ms: now_ms,
        items: items.clone(),
    });
    Ok(items)
}

fn filter_active_limits(items: &[WorkbuddyLogLimit], now_ms: i64) -> Vec<WorkbuddyLogLimit> {
    items
        .iter()
        .filter(|item| item.reset_at > now_ms)
        .cloned()
        .collect()
}

pub fn scan_workbuddy_log_limits_in_roots(
    roots: &[PathBuf],
    accounts: &[WorkbuddyAccount],
    now_ms: i64,
) -> Vec<WorkbuddyLogLimit> {
    let uid_to_account = build_uid_map(accounts);
    if uid_to_account.is_empty() {
        return Vec::new();
    }

    let mtime_cutoff = now_ms.saturating_sub(TWO_DAYS_MS);
    let mut raw_hits = Vec::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) => {
                logger::log_warn(&format!(
                    "[WorkBuddy LogLimits] 读取日志根目录失败 {}: {}",
                    root.display(),
                    err
                ));
                continue;
            }
        };

        for entry in entries.flatten() {
            let dir_path = entry.path();
            if !dir_path.is_dir() {
                continue;
            }
            let Some(dir_name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !is_date_dir_name(&dir_name) {
                continue;
            }

            let log_files = match fs::read_dir(&dir_path) {
                Ok(files) => files,
                Err(err) => {
                    logger::log_warn(&format!(
                        "[WorkBuddy LogLimits] 读取日志日期目录失败 {}: {}",
                        dir_path.display(),
                        err
                    ));
                    continue;
                }
            };

            for file_entry in log_files.flatten() {
                let file_path = file_entry.path();
                if !is_recent_log_file(&file_path, mtime_cutoff) {
                    continue;
                }
                let file_mtime_ms = fs::metadata(&file_path)
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(system_time_to_ms)
                    .unwrap_or(now_ms);
                parse_log_file(
                    &file_path,
                    file_mtime_ms,
                    &uid_to_account,
                    &mut raw_hits,
                );
            }
        }
    }

    aggregate_hits(raw_hits, now_ms)
}

fn build_uid_map(accounts: &[WorkbuddyAccount]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for account in accounts {
        if let Some(uid) = account.uid.as_deref() {
            let normalized = uid.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                map.entry(normalized).or_insert_with(|| account.id.clone());
            }
        }
    }
    map
}

fn is_date_dir_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok()
}

fn is_recent_log_file(path: &Path, mtime_cutoff: i64) -> bool {
    if !path.is_file() {
        return false;
    }
    let is_log = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    if !is_log {
        return false;
    }
    let Some(mtime_ms) = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_to_ms)
    else {
        return false;
    };
    mtime_ms >= mtime_cutoff
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    let first = needle[0];
    let mut idx = 0;
    let limit = haystack.len() - needle.len();
    while idx <= limit {
        let Some(offset) = haystack[idx..=limit].iter().position(|&b| b == first) else {
            return false;
        };
        let pos = idx + offset;
        if &haystack[pos..pos + needle.len()] == needle {
            return true;
        }
        idx = pos + 1;
    }
    false
}

fn contains_ascii_ignore_case(haystack: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() || haystack.len() < needle_lower.len() {
        return false;
    }
    let first_lower = needle_lower[0];
    let first_upper = first_lower.to_ascii_uppercase();
    let mut idx = 0;
    let limit = haystack.len() - needle_lower.len();
    while idx <= limit {
        let Some(offset) = haystack[idx..=limit]
            .iter()
            .position(|&b| b == first_lower || b == first_upper)
        else {
            return false;
        };
        let pos = idx + offset;
        if haystack[pos..pos + needle_lower.len()]
            .iter()
            .zip(needle_lower.iter())
            .all(|(&a, &b)| a.to_ascii_lowercase() == b)
        {
            return true;
        }
        idx = pos + 1;
    }
    false
}

fn bytes_may_contain_rate_limit(bytes: &[u8]) -> bool {
    contains_bytes(bytes, RATE_LIMIT_ZH_BYTES)
        || contains_ascii_ignore_case(bytes, RATE_LIMIT_EN_BYTES)
}

fn parse_log_file(
    path: &Path,
    file_mtime_ms: i64,
    uid_to_account: &HashMap<String, String>,
    out_hits: &mut Vec<RawLogLimitHit>,
) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            logger::log_warn(&format!(
                "[WorkBuddy LogLimits] 读取日志文件失败 {}: {}",
                path.display(),
                err
            ));
            return;
        }
    };

    if !bytes_may_contain_rate_limit(&bytes) {
        return;
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut current_uid: Option<String> = None;
    let mut session_models: HashMap<String, String> = HashMap::new();
    let mut last_ts_ms: Option<i64> = None;

    for raw_line in text.lines() {
        let line_owned;
        let line = if raw_line.contains('\0') {
            line_owned = raw_line.replace('\0', "");
            line_owned.as_str()
        } else {
            raw_line
        };

        if let Some(ts) = parse_line_timestamp_ms(line) {
            last_ts_ms = Some(ts);
        }

        if let Some(uid_update) = extract_uid_update(line) {
            current_uid = uid_update;
        }

        if line.contains("[ModelConfig]") {
            if let (Some(session_id), Some(model)) = (
                extract_kv_token(line, "sessionId="),
                extract_resolved_model(line),
            ) {
                session_models.insert(session_id, model);
            }
        }

        if is_false_positive_line(line) || !is_rate_limit_line(line) {
            continue;
        }

        let Some(ref uid) = current_uid else {
            continue;
        };
        let Some(account_id) = uid_to_account.get(&uid.to_ascii_lowercase()) else {
            continue;
        };
        let Some(reset_at) = parse_reset_at_ms(line) else {
            continue;
        };

        let occurred_at = parse_line_timestamp_ms(line)
            .or(last_ts_ms)
            .unwrap_or(file_mtime_ms);
        let session_id = extract_rate_limit_session_id(line).unwrap_or_default();
        let model = if session_id.is_empty() {
            String::new()
        } else {
            session_models
                .get(&session_id)
                .cloned()
                .unwrap_or_default()
        };

        out_hits.push(RawLogLimitHit {
            account_id: account_id.clone(),
            uid: uid.clone(),
            session_id,
            model,
            reset_at,
            occurred_at,
            message: sanitize_message(line),
        });
    }
}

fn is_false_positive_line(line: &str) -> bool {
    line.contains("handle_run_executable") || line.contains("RunExecutable")
}

fn is_rate_limit_line(line: &str) -> bool {
    line.contains("超出频率限制")
        || contains_ascii_ignore_case(line.as_bytes(), RATE_LIMIT_EN_BYTES)
}

fn local_naive_to_ms(ndt: NaiveDateTime) -> Option<i64> {
    Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|dt| dt.timestamp_millis())
        .or_else(|| Some(Utc.from_utc_datetime(&ndt).timestamp_millis()))
}

fn parse_line_timestamp_ms(line: &str) -> Option<i64> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let end = trimmed.find(']')?;
    let inner = trimmed[1..end].trim();
    if inner.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(inner) {
        return Some(dt.timestamp_millis());
    }

    let (base, millis) = split_trailing_millis(inner);
    let formats = [
        "%Y/%m/%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%m/%d/%Y, %I:%M:%S %p",
        "%m/%d/%Y %I:%M:%S %p",
    ];
    for fmt in formats {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(base, fmt) {
            return local_naive_to_ms(ndt).map(|ms| ms + millis);
        }
    }
    None
}

fn split_trailing_millis(input: &str) -> (&str, i64) {
    let Some(dot_idx) = input.rfind('.') else {
        return (input, 0);
    };
    let frac = &input[dot_idx + 1..];
    if frac.is_empty() || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return (input, 0);
    }
    let Ok(raw) = frac.parse::<i64>() else {
        return (input, 0);
    };
    let scaled = match frac.len() {
        1 => raw * 100,
        2 => raw * 10,
        _ => raw,
    };
    (&input[..dot_idx], scaled)
}

fn is_boundary_before(line: &str, idx: usize) -> bool {
    line[..idx]
        .chars()
        .next_back()
        .map_or(true, |ch| !ch.is_ascii_alphanumeric() && ch != '_')
}

fn read_token_value(rest: &str) -> &str {
    let trimmed = rest.trim_start_matches(|ch| ch == '"' || ch == '\'');
    let end = trimmed
        .find(|ch: char| {
            ch.is_whitespace()
                || matches!(ch, ',' | ';' | ')' | ']' | '"' | '\'' | '&')
        })
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

fn extract_kv_token(line: &str, key_eq: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find(key_eq) {
        let idx = search_from + rel;
        if is_boundary_before(line, idx) {
            let val = read_token_value(&line[idx + key_eq.len()..]).trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
        search_from = idx + key_eq.len();
    }
    None
}

/// Returns `Some(Some(uid))` when a valid `uid=<id>` appears,
/// `Some(None)` when an auth line explicitly clears `uid=<none>` / `<empty>`,
/// or `None` when the line does not carry a `uid=` state change.
fn extract_uid_update(line: &str) -> Option<Option<String>> {
    let raw_uid = extract_kv_token(line, "uid=")?;
    let lowered = raw_uid.to_ascii_lowercase();
    let is_placeholder = raw_uid.starts_with('<')
        || matches!(lowered.as_str(), "none" | "null" | "empty" | "undefined" | "n/a");
    if is_placeholder {
        if line.contains("AuthDoInitProbe") || line.contains("AuthenticationManager") {
            return Some(None);
        }
        return None;
    }
    Some(Some(raw_uid))
}

fn extract_resolved_model(line: &str) -> Option<String> {
    extract_kv_token(line, "resolved model=")
        .or_else(|| extract_kv_token(line, "resolvedModel="))
}

fn extract_rate_limit_session_id(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    if let Some(stripped) = trimmed.strip_suffix(')') {
        if let Some(open_idx) = stripped.rfind('(') {
            let inner = &stripped[open_idx + 1..];
            if !inner.contains("://") && !inner.chars().any(char::is_whitespace) {
                if let Some((req, sess)) = inner.split_once('/') {
                    if !req.is_empty() && !sess.is_empty() && !sess.contains('/') {
                        return Some(sess.to_string());
                    }
                }
            }
        }
    }
    extract_kv_token(line, "sessionId=")
}

fn parse_utc_offset_seconds(offset_str: Option<&str>) -> Option<i32> {
    let Some(raw) = offset_str.map(str::trim).filter(|s| !s.is_empty()) else {
        return Some(0);
    };
    let sign = if raw.starts_with('-') {
        -1
    } else if raw.starts_with('+') {
        1
    } else {
        return None;
    };
    let rest = &raw[1..];
    let (hours, mins) = if let Some((h, m)) = rest.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if rest.len() == 4 {
        (
            rest[..2].parse::<i32>().ok()?,
            rest[2..].parse::<i32>().ok()?,
        )
    } else {
        (rest.parse::<i32>().ok()?, 0)
    };
    if hours > 23 || mins > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + mins * 60))
}

fn find_reset_search_slice(line: &str) -> &str {
    if let Some(pos) = line.find("超出频率限制") {
        return &line[pos..];
    }
    let bytes = line.as_bytes();
    if bytes.len() >= RATE_LIMIT_EN_BYTES.len() {
        let first_lower = RATE_LIMIT_EN_BYTES[0];
        let first_upper = first_lower.to_ascii_uppercase();
        let limit = bytes.len() - RATE_LIMIT_EN_BYTES.len();
        for pos in 0..=limit {
            let b = bytes[pos];
            if (b == first_lower || b == first_upper)
                && bytes[pos..pos + RATE_LIMIT_EN_BYTES.len()]
                    .iter()
                    .zip(RATE_LIMIT_EN_BYTES.iter())
                    .all(|(&a, &target)| a.to_ascii_lowercase() == target)
            {
                return &line[pos..];
            }
        }
    }
    // Fallback for unit tests calling parse_reset_at_ms directly without the prefix.
    let mut rest = line.trim_start();
    while rest.starts_with('[') {
        let Some(end) = rest.find(']') else {
            break;
        };
        rest = rest[end + 1..].trim_start();
    }
    rest
}

fn build_naive_datetime_from_caps(caps: &regex::Captures<'_>) -> Option<NaiveDateTime> {
    let year: i32 = caps.get(1)?.as_str().parse().ok()?;
    let month: u32 = caps.get(2)?.as_str().parse().ok()?;
    let day: u32 = caps.get(3)?.as_str().parse().ok()?;
    let hour: u32 = caps.get(4)?.as_str().parse().ok()?;
    let minute: u32 = caps.get(5)?.as_str().parse().ok()?;
    let second: u32 = caps
        .get(6)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    date.and_hms_opt(hour, minute, second)
}

pub fn parse_reset_at_ms(line: &str) -> Option<i64> {
    let target = find_reset_search_slice(line);

    if let Some(caps) = RE_ISO_TZ.captures(target) {
        if let Ok(dt) = DateTime::parse_from_rfc3339(caps.get(1)?.as_str()) {
            return Some(dt.timestamp_millis());
        }
    }

    if let Some(caps) = RE_UTC_OFFSET.captures(target) {
        let ndt = build_naive_datetime_from_caps(&caps)?;
        let offset_secs = parse_utc_offset_seconds(caps.get(7).map(|m| m.as_str()))?;
        let offset = FixedOffset::east_opt(offset_secs)?;
        return offset
            .from_local_datetime(&ndt)
            .single()
            .map(|dt| dt.timestamp_millis());
    }

    if let Some(caps) = RE_LOCAL_DATETIME.captures(target) {
        let ndt = build_naive_datetime_from_caps(&caps)?;
        return local_naive_to_ms(ndt);
    }

    None
}

fn sanitize_message(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= MAX_MESSAGE_CHARS {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(MAX_MESSAGE_CHARS).collect();
    format!("{truncated}...")
}

fn aggregate_hits(mut hits: Vec<RawLogLimitHit>, now_ms: i64) -> Vec<WorkbuddyLogLimit> {
    hits.retain(|hit| hit.reset_at > now_ms);
    if hits.is_empty() {
        return Vec::new();
    }

    // Step 1: Deduplicate events sharing (account_id, session_id, model, reset_at)
    // whose occurred_at timestamps are within 15 seconds.
    hits.sort_by(|a, b| {
        a.account_id
            .cmp(&b.account_id)
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.model.cmp(&b.model))
            .then_with(|| a.reset_at.cmp(&b.reset_at))
            .then_with(|| a.occurred_at.cmp(&b.occurred_at))
    });

    let mut deduped: Vec<WorkbuddyLogLimit> = Vec::new();
    for hit in hits {
        let merged = if let Some(last) = deduped.last_mut() {
            if last.account_id == hit.account_id
                && last.session_id == hit.session_id
                && last.model == hit.model
                && last.reset_at == hit.reset_at
                && hit.occurred_at - last.last_seen_at <= DEDUP_WINDOW_MS
            {
                last.hit_count = last.hit_count.saturating_add(1);
                last.first_seen_at = last.first_seen_at.min(hit.occurred_at);
                last.last_seen_at = last.last_seen_at.max(hit.occurred_at);
                if !hit.message.is_empty() {
                    last.message = hit.message.clone();
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        if !merged {
            deduped.push(WorkbuddyLogLimit {
                account_id: hit.account_id,
                uid: hit.uid,
                session_id: hit.session_id,
                model: hit.model,
                reset_at: hit.reset_at,
                first_seen_at: hit.occurred_at,
                last_seen_at: hit.occurred_at,
                hit_count: 1,
                message: hit.message,
                source: SOURCE_CLIENT_LOG.to_string(),
            });
        }
    }

    // Step 2: Aggregate by (account_id, model), keeping the entry with max reset_at.
    let mut per_account_model: HashMap<(String, String), WorkbuddyLogLimit> = HashMap::new();
    for entry in deduped {
        let key = (entry.account_id.clone(), entry.model.clone());
        match per_account_model.get(&key) {
            Some(existing)
                if existing.reset_at > entry.reset_at
                    || (existing.reset_at == entry.reset_at
                        && existing.last_seen_at > entry.last_seen_at) => {}
            _ => {
                per_account_model.insert(key, entry);
            }
        }
    }

    let mut result: Vec<WorkbuddyLogLimit> = per_account_model.into_values().collect();
    result.sort_by(|a, b| {
        b.reset_at
            .cmp(&a.reset_at)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| a.model.cmp(&b.model))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mock_account(id: &str, uid: Option<&str>) -> WorkbuddyAccount {
        WorkbuddyAccount {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            uid: uid.map(str::to_string),
            nickname: None,
            enterprise_id: None,
            enterprise_name: None,
            tags: None,
            access_token: "token".to_string(),
            refresh_token: None,
            token_type: None,
            expires_at: None,
            domain: None,
            plan_type: None,
            dosage_notify_code: None,
            dosage_notify_zh: None,
            dosage_notify_en: None,
            payment_type: None,
            quota_raw: None,
            auth_raw: None,
            profile_raw: None,
            usage_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: None,
            checkin_rewards: None,
            created_at: 0,
            last_used: 0,
            last_switched_at: None,
            web_session_enabled: None,
        }
    }

    fn format_utc_plus_8(ts_ms: i64) -> String {
        let offset = FixedOffset::east_opt(8 * 3600).unwrap();
        let dt = offset.timestamp_millis_opt(ts_ms).unwrap();
        dt.format("%Y-%m-%d %H:%M:%S UTC+8").to_string()
    }

    fn format_bracket_ts(ts_ms: i64) -> String {
        let dt = Local.timestamp_millis_opt(ts_ms).unwrap();
        dt.format("%Y/%m/%d %H:%M:%S%.3f").to_string()
    }

    fn today_date_dir() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    #[test]
    fn parses_timestamps_and_reset_formats() {
        let ts = parse_line_timestamp_ms("[2026/11/16 10:04:29.015] [Info] hello");
        assert!(ts.is_some());

        let am_pm_ts = parse_line_timestamp_ms("[11/23/2026, 5:03:22 PM.002] [Info] hello");
        assert!(am_pm_ts.is_some());

        let reset = parse_reset_at_ms(
            "[2026/11/16 10:04:29.015] [Error] 429 您的使用量已超出频率限制，将在 2026-11-16 14:03:01 UTC+8 重置 (req-1/sess-1)",
        )
        .expect("must parse UTC+8 resetAt");
        let expected = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 16, 14, 3, 1)
            .unwrap()
            .timestamp_millis();
        assert_eq!(reset, expected);
    }

    #[test]
    fn matches_rate_limit_with_uid_and_model_mapping() {
        let dir = TempDir::new().unwrap();
        let date_dir = dir.path().join(today_date_dir());
        fs::create_dir_all(&date_dir).unwrap();

        let now_ms = current_time_ms();
        let reset_ms = ((now_ms + 3_600_000) / 1000) * 1000;
        let log_ts = format_bracket_ts(now_ms - 5_000);
        let reset_str = format_utc_plus_8(reset_ms);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            format!(
                "[{log_ts}] [Info] [AuthenticationManager] [AuthDoInitProbe] uid=uid-alice-123 hasAccessToken=true\n\
                 [{log_ts}] [Info] [ModelConfig] sessionId=sess-001 resolved model=claude-sonnet-4-5\n\
                 [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-99/sess-001)\n"
            )
            .as_bytes(),
        );
        // Include a NUL byte to verify raw byte pre-filter + lossy parsing still succeeds.
        bytes.extend_from_slice(b"\0\0");
        fs::write(date_dir.join("main.log"), &bytes).unwrap();

        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[dir.path().to_path_buf()],
            &accounts,
            now_ms,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].account_id, "workbuddy_alice");
        assert_eq!(results[0].uid, "uid-alice-123");
        assert_eq!(results[0].session_id, "sess-001");
        assert_eq!(results[0].model, "claude-sonnet-4-5");
        assert_eq!(results[0].reset_at, reset_ms);
        assert_eq!(results[0].hit_count, 1);
        assert_eq!(results[0].source, "client_log");
    }

    #[test]
    fn discards_entries_without_uid_or_unmatched_account() {
        let dir = TempDir::new().unwrap();
        let date_dir = dir.path().join(today_date_dir());
        fs::create_dir_all(&date_dir).unwrap();

        let now_ms = current_time_ms();
        let reset_ms = ((now_ms + 3_600_000) / 1000) * 1000;
        let log_ts = format_bracket_ts(now_ms - 5_000);
        let reset_str = format_utc_plus_8(reset_ms);

        let content = format!(
            "[{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-1/sess-no-uid)\n\
             [{log_ts}] [Info] [AuthDoInitProbe] uid=uid-stranger\n\
             [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-2/sess-stranger)\n"
        );
        fs::write(date_dir.join("renderer.log"), content).unwrap();

        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[dir.path().to_path_buf()],
            &accounts,
            now_ms,
        );
        assert!(results.is_empty());
    }

    #[test]
    fn skips_non_date_dirs_like_sandbox_and_false_positive_lines() {
        let dir = TempDir::new().unwrap();
        let sandbox_dir = dir.path().join("sandbox");
        let date_dir = dir.path().join(today_date_dir());
        fs::create_dir_all(&sandbox_dir).unwrap();
        fs::create_dir_all(&date_dir).unwrap();

        let now_ms = current_time_ms();
        let reset_ms = ((now_ms + 3_600_000) / 1000) * 1000;
        let log_ts = format_bracket_ts(now_ms - 5_000);
        let reset_str = format_utc_plus_8(reset_ms);

        let content = format!(
            "[{log_ts}] [Info] [AuthDoInitProbe] uid=uid-alice-123\n\
             [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-1/sess-1)\n"
        );
        fs::write(sandbox_dir.join("sandbox.log"), &content).unwrap();

        let fp_content = format!(
            "[{log_ts}] [Info] [AuthDoInitProbe] uid=uid-alice-123\n\
             [{log_ts}] [Info] handle_run_executable output: 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-1/sess-1)\n\
             [{log_ts}] [Info] RunExecutable stdout: usage exceeds frequency limit {reset_str}\n"
        );
        fs::write(date_dir.join("daemon.log"), fp_content).unwrap();

        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[dir.path().to_path_buf()],
            &accounts,
            now_ms,
        );
        assert!(results.is_empty());
    }

    #[test]
    fn deduplicates_five_lines_into_single_event_with_hit_count_five() {
        let dir = TempDir::new().unwrap();
        let date_dir = dir.path().join(today_date_dir());
        fs::create_dir_all(&date_dir).unwrap();

        let now_ms = current_time_ms();
        let reset_ms = ((now_ms + 7_200_000) / 1000) * 1000;
        let reset_str = format_utc_plus_8(reset_ms);

        let mut lines = format!(
            "[{}] [Info] [AuthDoInitProbe] uid=uid-alice-123\n\
             [{}] [Info] [ModelConfig] sessionId=sess-dup resolved model=hy4free\n",
            format_bracket_ts(now_ms - 20_000),
            format_bracket_ts(now_ms - 19_000),
        );
        let first_hit_ms = now_ms - 10_000;
        for i in 0..5 {
            let ts = format_bracket_ts(first_hit_ms + i * 1_000);
            lines.push_str(&format!(
                "[{ts}] [Error] 429 您的使用量已超出频率限制，将在 {reset_str} 重置 (req-{i}/sess-dup)\n"
            ));
        }
        fs::write(date_dir.join("main.log"), lines).unwrap();

        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[dir.path().to_path_buf()],
            &accounts,
            now_ms,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hit_count, 5);
        assert_eq!(results[0].first_seen_at, first_hit_ms);
        assert_eq!(results[0].last_seen_at, first_hit_ms + 4_000);
        assert_eq!(results[0].model, "hy4free");
    }

    #[test]
    fn filters_out_expired_reset_at_and_keeps_max_reset_at_per_model() {
        let dir = TempDir::new().unwrap();
        let date_dir = dir.path().join(today_date_dir());
        fs::create_dir_all(&date_dir).unwrap();

        let now_ms = current_time_ms();
        let expired_reset_ms = ((now_ms - 60_000) / 1000) * 1000;
        let active_early_ms = ((now_ms + 1_800_000) / 1000) * 1000;
        let active_later_ms = ((now_ms + 5_400_000) / 1000) * 1000;
        let log_ts = format_bracket_ts(now_ms - 5_000);

        let content = format!(
            "[{log_ts}] [Info] [AuthDoInitProbe] uid=uid-alice-123\n\
             [{log_ts}] [Info] [ModelConfig] sessionId=sess-exp resolved model=expired-model\n\
             [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {} 重置 (req-1/sess-exp)\n\
             [{log_ts}] [Info] [ModelConfig] sessionId=sess-1 resolved model=gpt-5\n\
             [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {} 重置 (req-2/sess-1)\n\
             [{log_ts}] [Info] [ModelConfig] sessionId=sess-2 resolved model=gpt-5\n\
             [{log_ts}] [Error] 429 您的使用量已超出频率限制，将在 {} 重置 (req-3/sess-2)\n",
            format_utc_plus_8(expired_reset_ms),
            format_utc_plus_8(active_early_ms),
            format_utc_plus_8(active_later_ms),
        );
        fs::write(date_dir.join("main.log"), content).unwrap();

        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[dir.path().to_path_buf()],
            &accounts,
            now_ms,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].model, "gpt-5");
        assert_eq!(results[0].reset_at, active_later_ms);
    }

    #[test]
    fn returns_empty_safely_when_root_directory_does_not_exist() {
        let missing = PathBuf::from("/tmp/nonexistent-workbuddy-logs-dir-3214");
        let accounts = vec![mock_account("workbuddy_alice", Some("uid-alice-123"))];
        let results = scan_workbuddy_log_limits_in_roots(
            &[missing],
            &accounts,
            current_time_ms(),
        );
        assert!(results.is_empty());
    }
}
