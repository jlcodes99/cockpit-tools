//! Multi-core + WAL-aware strictly read-only Antigravity telemetry scanner.
//!
//! Reconciles all local telemetry sources with zero double-counting and zero writes/locks
//! inside `~/.gemini/antigravity`:
//!   1. `conversation_summaries.db`
//!   2. `annotations/*.pbtxt`
//!   3. `brain/*/.system_generated/swarm/members.json` & `logs/transcript.jsonl`
//!   4. `.token-monitor/rpc-cache/v1/<id>/manifest.json` & `usage.jsonl`
//!   5. `conversations/<id>.db` (primary source, opened `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`)
//!
//! Performance architecture:
//! - Uses `DirEntry::metadata()` during single-pass directory scans (zero `CreateFileW` / AV scans).
//! - Uses compact minute-bucketed model rollups (`scan-cache-v3`, ~50x smaller than per-turn JSON)
//!   with string interning + persisted RPC/annotation/swarm metadata caches.
//! - Extracts full per-turn `Vec<TurnUsageRecord>` lazily on demand in ~1–2 ms only when a single
//!   conversation's detail modal is opened (`build_conversation_detail`).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use super::models::{
    canonicalize_model_id, format_model_display_name, is_builtin_enum_u64, resolve_model_info,
    ModelResolutionInput, ResolvedModelInfo,
};
use super::protobuf::{
    estimate_tokens_from_bytes, extract_readable_utf8_bytes, format_utc_ms, format_utc_secs_nanos,
    parse_gen_metadata_proto, parse_gen_metadata_slim, parse_step_metadata_proto,
    parse_step_metadata_slim, ParsedGenMetadataProto, SlimGenMetadataProto, SlimStepMetadataProto,
};
use super::types::{
    ChildConversationRef, ConversationDetail, ConversationKind, ConversationModelSlice,
    ConversationSummaryItem, DataSourceKind, EstimatedTokenTotals, ExactTokenTotals,
    ResolvedTurnGroup, ScanMetadata, ScanResult, ScannedConversation, TurnEstimatedTotals,
    TurnUsageRecord,
};

const READ_ONLY_SQLITE_FLAGS: OpenFlags =
    OpenFlags::SQLITE_OPEN_READ_ONLY.union(OpenFlags::SQLITE_OPEN_NO_MUTEX);

const CACHE_SCHEMA_VERSION: u32 = 3;
const RPC_HARVEST_HEAD_BYTES: u64 = 8 * 1024;
const NO_TIMESTAMP_SENTINEL: i64 = i64::MIN;
pub const UNASSIGNED_WORKSPACE_KEY: &str = "__none__";
const UNASSIGNED_WORKSPACE_NAME: &str = "(Unassigned)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWorkspace {
    pub workspace_uri: String,
    pub workspace_path: String,
    pub workspace_name: String,
    pub workspace_key: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprint {
    pub db_size: u64,
    pub db_mtime_ms: u64,
    pub wal_size: u64,
    pub wal_mtime_ms: u64,
    pub rpc_size: u64,
    pub rpc_mtime_ms: u64,
    pub transcript_size: u64,
    pub transcript_mtime_ms: u64,
}

impl FileFingerprint {
    #[inline]
    fn weight_bytes(&self) -> u64 {
        self.db_size
            .saturating_add(self.wal_size)
            .saturating_add(self.rpc_size)
            .saturating_add(self.transcript_size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRawTurn {
    pub step_index: usize,
    pub timestamp: Option<String>,
    pub friendly_model: Option<String>,
    pub raw_model_enum: Option<String>,
    pub response_model: Option<String>,
    pub raw_api_provider: Option<String>,
    pub is_exact: bool,
    pub source: DataSourceKind,
    pub status: i32,
    pub exact: ExactTokenTotals,
    pub estimated: TurnEstimatedTotals,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
struct RawModelKey {
    friendly_model: Option<String>,
    raw_model_enum: Option<String>,
    response_model: Option<String>,
    raw_api_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedTurnGroup {
    max_ts_ms: Option<i64>,
    model_key: RawModelKey,
    exact_turn_count: usize,
    estimated_turn_count: usize,
    exact: ExactTokenTotals,
    estimated: TurnEstimatedTotals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedConversationTelemetry {
    fingerprint: FileFingerprint,
    data_source: DataSourceKind,
    db_step_count: usize,
    created_at_ms: Option<i64>,
    is_native_swarm_worker: bool,
    groups: Vec<CachedTurnGroup>,
}

/// Compact 14-tuple struct for `CachedTurnGroup` (serializes as a compact JSON integer array):
/// `[max_ts_ms_or_min, friendly_idx, enum_idx, resp_idx, prov_idx,
///   exact_turns, est_turns, input, output, thinking, cache_read, cache_write, est_in, est_out]`
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactGroupTuple(
    i64,
    u32,
    u32,
    u32,
    u32,
    usize,
    usize,
    u64,
    u64,
    u64,
    u64,
    u64,
    u64,
    u64,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedConvEntry {
    fp: FileFingerprint,
    src: DataSourceKind,
    steps: usize,
    cat: Option<i64>,
    #[serde(default)]
    sw: bool,
    g: Vec<CompactGroupTuple>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedAnnotationEntry {
    s: u64,
    m: u64,
    t: Option<String>,
    v: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRpcEntry {
    ms: u64,
    mm: u64,
    us: u64,
    um: u64,
    hu: bool,
    t: Option<String>,
    sc: usize,
    lm: Option<i64>,
    #[serde(default)]
    en: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSwarmWorker {
    wid: String,
    ag: String,
    mh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSwarmDirEntry {
    s: u64,
    m: u64,
    cid: Option<String>,
    w: Vec<PersistedSwarmWorker>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedScanCacheFile {
    schema_version: u32,
    #[serde(default)]
    string_pool: Vec<String>,
    #[serde(default)]
    entries: HashMap<String, PersistedConvEntry>,
    #[serde(default)]
    dynamic_enum_map: HashMap<String, String>,
    #[serde(default)]
    annotations: HashMap<String, PersistedAnnotationEntry>,
    #[serde(default)]
    rpc_sessions: HashMap<String, PersistedRpcEntry>,
    #[serde(default)]
    swarm_dirs: HashMap<String, PersistedSwarmDirEntry>,
}

#[derive(Default)]
struct StringInterner {
    pool: Vec<String>,
    index: HashMap<String, u32>,
}

impl StringInterner {
    fn intern(&mut self, val: Option<&str>) -> u32 {
        let Some(s) = val.filter(|x| !x.is_empty()) else {
            return 0;
        };
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let next_id = (self.pool.len() + 1) as u32;
        self.pool.push(s.to_string());
        self.index.insert(s.to_string(), next_id);
        next_id
    }
}

#[inline]
fn pool_lookup(pool: &[String], idx: u32) -> Option<String> {
    if idx == 0 {
        None
    } else {
        pool.get((idx - 1) as usize).cloned()
    }
}

#[derive(Debug, Clone, Default)]
struct SummaryDbRow {
    conversation_id: String,
    title: String,
    preview: String,
    step_count: usize,
    last_modified_ms: Option<i64>,
    workspace_uris: String,
    status: String,
    project_id: String,
    agent_name: String,
    parent_conversation_id: String,
    nesting_depth: usize,
}

#[derive(Debug, Clone, Default)]
struct AnnotationMeta {
    title: Option<String>,
    last_view_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct AnnotationCacheEntry {
    size: u64,
    mtime_ms: u64,
    meta: AnnotationMeta,
}

#[derive(Debug, Clone, Default)]
struct RpcSessionMeta {
    title: Option<String>,
    step_count: usize,
    last_modified_ms: Option<i64>,
    has_usage: bool,
    usage_size: u64,
    usage_mtime_ms: u64,
    enums: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct RpcMetaCacheEntry {
    manifest_size: u64,
    manifest_mtime_ms: u64,
    usage_size: u64,
    usage_mtime_ms: u64,
    meta: RpcSessionMeta,
}

#[derive(Debug, Clone)]
struct SwarmWorkerMeta {
    coordinator_id: String,
    agent_name: String,
    model_hint: Option<String>,
}

#[derive(Debug, Clone)]
struct SwarmDirCacheEntry {
    size: u64,
    mtime_ms: u64,
    coordinator_id: Option<String>,
    workers: Vec<(String, SwarmWorkerMeta)>,
}

#[derive(Debug, Clone)]
struct ExtractionTask {
    conversation_id: String,
    fingerprint: FileFingerprint,
    db_path: Option<PathBuf>,
    rpc_usage_path: Option<PathBuf>,
    transcript_path: Option<PathBuf>,
}

struct ExtractionOutcome {
    conversation_id: String,
    telemetry: CachedConversationTelemetry,
    warning: Option<String>,
}

pub fn get_default_antigravity_data_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("ANTIGRAVITY_DATA_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gemini")
        .join("antigravity")
}

pub fn resolve_antigravity_data_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(p) = override_dir {
        if !p.as_os_str().is_empty() {
            return p.to_path_buf();
        }
    }
    get_default_antigravity_data_dir()
}

pub fn get_workspace_key_from_path(workspace_path: &str) -> String {
    let trimmed = workspace_path.trim();
    if trimmed.is_empty() {
        UNASSIGNED_WORKSPACE_KEY.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn format_utc_datetime(dt: DateTime<Utc>) -> String {
    format!(
        "{}.{:03}Z",
        dt.format("%Y-%m-%dT%H:%M:%S"),
        dt.timestamp_subsec_millis()
    )
}

#[inline]
fn metadata_to_size_mtime(meta: &std::fs::Metadata) -> (u64, u64) {
    let size = meta.len();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (size, mtime_ms)
}

/// Fast attribute-only stat (`GetFileAttributesExW` on Windows — never opens a file handle or triggers AV scan).
#[inline]
pub fn safe_stat_wal(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some(metadata_to_size_mtime(&meta))
}

#[inline]
fn dir_entry_size_mtime(entry: &std::fs::DirEntry) -> Option<(u64, u64)> {
    let meta = entry.metadata().ok()?;
    Some(metadata_to_size_mtime(&meta))
}

fn percent_decode_utf8(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn normalize_single_workspace_uri(raw_uri: &str) -> (String, String) {
    let uri = raw_uri.trim();
    if uri.is_empty() {
        return (String::new(), String::new());
    }

    if uri.len() >= 7 && uri[..7].eq_ignore_ascii_case("file://") {
        let after_scheme = &uri[7..];
        let without_qf = after_scheme.split(['?', '#']).next().unwrap_or("");
        let path_part =
            if without_qf.len() >= 10 && without_qf[..10].eq_ignore_ascii_case("localhost/") {
                &without_qf[9..]
            } else {
                without_qf
            };

        if path_part.starts_with('/') {
            let decoded = percent_decode_utf8(path_part);
            let db = decoded.as_bytes();
            if db.len() >= 3
                && db[0] == b'/'
                && db[1].is_ascii_alphabetic()
                && db[2] == b':'
                && (db.len() == 3 || db[3] == b'/' || db[3] == b'\\')
            {
                let drive = (db[1] as char).to_ascii_uppercase();
                let tail = decoded[3..].replace('/', "\\");
                let trimmed_tail = tail.trim_end_matches('\\');
                let win_path = if trimmed_tail.is_empty() {
                    format!("{}:\\", drive)
                } else {
                    format!("{}:{}", drive, trimmed_tail)
                };
                return (uri.to_string(), win_path);
            }

            let posix_path = if decoded == "/" {
                "/".to_string()
            } else {
                let t = decoded.trim_end_matches('/');
                if t.is_empty() {
                    "/".to_string()
                } else {
                    t.to_string()
                }
            };
            return (uri.to_string(), posix_path);
        } else {
            let decoded = percent_decode_utf8(path_part);
            let clean = decoded.replace('/', "\\");
            let trimmed = clean.trim_matches('\\');
            if trimmed.is_empty() {
                return (String::new(), String::new());
            }
            return (uri.to_string(), format!("\\\\{}", trimmed));
        }
    }

    let decoded = percent_decode_utf8(uri);
    let db = decoded.as_bytes();
    if db.len() >= 2 && db[0].is_ascii_alphabetic() && db[1] == b':' {
        let drive = (db[0] as char).to_ascii_uppercase();
        let tail = decoded[2..].replace('/', "\\");
        let trimmed_tail = tail.trim_end_matches('\\');
        let win_path = if trimmed_tail.is_empty() {
            format!("{}:\\", drive)
        } else {
            format!("{}:{}", drive, trimmed_tail)
        };
        return (uri.to_string(), win_path);
    }
    if decoded.starts_with('/') {
        let posix = if decoded == "/" {
            "/".to_string()
        } else {
            decoded.trim_end_matches('/').to_string()
        };
        return (uri.to_string(), posix);
    }

    (uri.to_string(), decoded)
}

fn basename_cross_os(workspace_path: &str) -> String {
    if workspace_path.is_empty() {
        return UNASSIGNED_WORKSPACE_NAME.to_string();
    }
    if workspace_path == "/" {
        return "/".to_string();
    }
    let segments: Vec<&str> = workspace_path
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(&last) = segments.last() {
        if last.len() == 2 && last.as_bytes()[1] == b':' {
            format!("{}\\", last)
        } else {
            last.to_string()
        }
    } else {
        workspace_path.to_string()
    }
}

pub fn decode_workspace_uris(raw_uris: &str) -> DecodedWorkspace {
    let trimmed = raw_uris.trim();
    if trimmed.is_empty() || trimmed == "[]" || trimmed == "null" {
        return DecodedWorkspace {
            workspace_uri: String::new(),
            workspace_path: String::new(),
            workspace_name: UNASSIGNED_WORKSPACE_NAME.to_string(),
            workspace_key: UNASSIGNED_WORKSPACE_KEY.to_string(),
        };
    }

    let first_uri: Option<String> = if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
            arr.into_iter().find(|s| !s.trim().is_empty())
        } else {
            trimmed
                .trim_matches(['[', ']'])
                .split(',')
                .map(|s| s.trim().trim_matches('"').trim())
                .find(|s| !s.is_empty())
                .map(ToString::to_string)
        }
    } else {
        Some(trimmed.trim_matches('"').to_string())
    };

    let Some(candidate) = first_uri else {
        return DecodedWorkspace {
            workspace_uri: String::new(),
            workspace_path: String::new(),
            workspace_name: UNASSIGNED_WORKSPACE_NAME.to_string(),
            workspace_key: UNASSIGNED_WORKSPACE_KEY.to_string(),
        };
    };

    let (workspace_uri, workspace_path) = normalize_single_workspace_uri(&candidate);
    if workspace_path.is_empty() {
        return DecodedWorkspace {
            workspace_uri: String::new(),
            workspace_path: String::new(),
            workspace_name: UNASSIGNED_WORKSPACE_NAME.to_string(),
            workspace_key: UNASSIGNED_WORKSPACE_KEY.to_string(),
        };
    }

    let workspace_name = basename_cross_os(&workspace_path);
    let workspace_key = get_workspace_key_from_path(&workspace_path);

    DecodedWorkspace {
        workspace_uri,
        workspace_path,
        workspace_name,
        workspace_key,
    }
}

pub fn parse_timestamp_ms(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(val) = trimmed.parse::<u64>() {
            if val >= 100_000_000_000 {
                return i64::try_from(val).ok().filter(|&ms| ms > 0);
            } else if val > 0 {
                return i64::try_from(val)
                    .ok()
                    .and_then(|s| s.checked_mul(1_000))
                    .filter(|&ms| ms > 0);
            }
        }
        return None;
    }

    let without_mono = trimmed.split(" m=").next().unwrap_or(trimmed).trim();
    let without_utc = without_mono
        .strip_suffix(" UTC")
        .unwrap_or(without_mono)
        .trim();

    if let Ok(dt) = DateTime::parse_from_rfc3339(without_utc) {
        let ms = dt.timestamp_millis();
        return (ms > 0).then_some(ms);
    }

    if without_utc.len() >= 19 && without_utc.as_bytes()[10] == b' ' {
        let mut iso = String::with_capacity(without_utc.len() + 4);
        iso.push_str(&without_utc[..10]);
        iso.push('T');
        let rest = without_utc[11..].trim();

        let normalized_rest = if let Some((time_part, tz_part)) = rest.split_once(' ') {
            let tz = tz_part.trim();
            if (tz.starts_with('+') || tz.starts_with('-')) && tz.len() == 5 && !tz.contains(':') {
                format!("{}{}:{}", time_part, &tz[..3], &tz[3..])
            } else {
                format!("{}{}", time_part, tz)
            }
        } else if (rest.len() > 5)
            && (rest.as_bytes()[rest.len() - 5] == b'+' || rest.as_bytes()[rest.len() - 5] == b'-')
            && rest[rest.len() - 4..].bytes().all(|b| b.is_ascii_digit())
        {
            let split_at = rest.len() - 5;
            format!(
                "{}{}:{}",
                &rest[..split_at],
                &rest[split_at..split_at + 3],
                &rest[split_at + 3..]
            )
        } else {
            rest.to_string()
        };

        iso.push_str(&normalized_rest);

        let after_t = &iso[11..];
        let has_tz = after_t.ends_with('Z')
            || after_t.ends_with('z')
            || after_t.contains('+')
            || after_t.contains('-');
        if !has_tz {
            iso.push('Z');
        }

        if let Ok(dt) = DateTime::parse_from_rfc3339(&iso) {
            let ms = dt.timestamp_millis();
            return (ms > 0).then_some(ms);
        }
    }

    None
}

pub fn normalize_timestamp(raw: &str) -> Option<String> {
    parse_timestamp_ms(raw).and_then(format_utc_ms)
}

fn unescape_pbtxt_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                    continue;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                    continue;
                }
                b'\'' => {
                    out.push(b'\'');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                b'x' | b'X' if i + 3 < bytes.len() => {
                    let hi = (bytes[i + 2] as char).to_digit(16);
                    let lo = (bytes[i + 3] as char).to_digit(16);
                    if let (Some(h), Some(l)) = (hi, lo) {
                        out.push(((h << 4) | l) as u8);
                        i += 4;
                        continue;
                    }
                }
                b'0'..=b'7' => {
                    let mut val: u32 = 0;
                    let mut consumed = 0;
                    while consumed < 3
                        && i + 1 + consumed < bytes.len()
                        && matches!(bytes[i + 1 + consumed], b'0'..=b'7')
                    {
                        val = (val << 3) | ((bytes[i + 1 + consumed] - b'0') as u32);
                        consumed += 1;
                    }
                    out.push((val & 0xff) as u8);
                    i += 1 + consumed;
                    continue;
                }
                other => {
                    out.push(other);
                    i += 2;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn annotation_title_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"title\s*:\s*"((?:\\.|[^"\\])*)""#).expect("valid regex"))
}

fn annotation_view_time_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"last_user_view_time\s*:\s*\{[^}]*?seconds\s*:\s*(\d+)(?:[^}]*?nanos\s*:\s*(\d+))?",
        )
        .expect("valid regex")
    })
}

fn parse_annotation_pbtxt(content: &str) -> AnnotationMeta {
    let title = annotation_title_regex()
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| unescape_pbtxt_string(m.as_str()).trim().to_string())
        .filter(|s| !s.is_empty());

    let last_view_ms = annotation_view_time_regex()
        .captures(content)
        .and_then(|c| {
            let secs = c.get(1)?.as_str().parse::<i64>().ok()?;
            let nanos = c
                .get(2)
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .unwrap_or(0);
            if secs <= 0 {
                return None;
            }
            secs.checked_mul(1_000)?
                .checked_add((nanos / 1_000_000) as i64)
        });

    AnnotationMeta {
        title,
        last_view_ms,
    }
}

fn json_u64(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(i) = v.as_i64() {
        return if i >= 0 { Some(i as u64) } else { Some(0) };
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            return Some(f.round() as u64);
        }
    }
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        if let Ok(n) = trimmed.parse::<u64>() {
            return Some(n);
        }
        if let Ok(i) = trimmed.parse::<i64>() {
            return Some(if i >= 0 { i as u64 } else { 0 });
        }
    }
    None
}

fn json_nonempty_str<'a>(v: &'a serde_json::Value) -> Option<&'a str> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

pub fn parse_rpc_usage_jsonl_content(content: &str) -> Vec<CachedRawTurn> {
    let mut by_step: HashMap<usize, CachedRawTurn> = HashMap::new();
    let mut fallback_seq = 0usize;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(rec_type) = obj.get("recordType").and_then(|v| v.as_str()) {
            if rec_type != "usage" {
                continue;
            }
        }

        let raw_obj = obj.get("raw");
        let chat_model = raw_obj.and_then(|r| r.get("chatModel"));
        let raw_usage = chat_model.and_then(|cm| cm.get("usage"));
        let custom_meta = chat_model.and_then(|cm| cm.get("customMetadata"));

        let step_index = raw_obj
            .and_then(|r| r.get("stepIndices"))
            .and_then(|arr| arr.as_array())
            .and_then(|arr| arr.first())
            .and_then(json_u64)
            .or_else(|| obj.get("stepIndex").and_then(json_u64))
            .or_else(|| obj.get("sequence").and_then(json_u64))
            .map(|n| n as usize)
            .unwrap_or(fallback_seq);
        fallback_seq = fallback_seq.saturating_add(1);

        let thinking_tokens = obj
            .get("reasoningTokens")
            .and_then(json_u64)
            .or_else(|| obj.get("thinkingTokens").and_then(json_u64))
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("thinkingOutputTokens"))
                    .and_then(json_u64)
            })
            .unwrap_or(0);

        let input_tokens = obj
            .get("inputTokens")
            .and_then(json_u64)
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("inputTokens"))
                    .and_then(json_u64)
            })
            .unwrap_or(0);

        let cache_read_tokens = obj
            .get("cacheReadTokens")
            .and_then(json_u64)
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("cacheReadTokens"))
                    .and_then(json_u64)
            })
            .unwrap_or(0);

        let cache_write_tokens = obj
            .get("cacheWriteTokens")
            .and_then(json_u64)
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("cacheWriteTokens"))
                    .and_then(json_u64)
            })
            .unwrap_or(0);

        let output_tokens = obj
            .get("outputTokens")
            .and_then(json_u64)
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("responseOutputTokens"))
                    .and_then(json_u64)
            })
            .unwrap_or_else(|| {
                let raw_combined = raw_usage
                    .and_then(|u| u.get("outputTokens"))
                    .and_then(json_u64)
                    .unwrap_or(0);
                raw_combined.saturating_sub(thinking_tokens)
            });

        let total_tokens = input_tokens
            .saturating_add(output_tokens)
            .saturating_add(thinking_tokens)
            .saturating_add(cache_read_tokens)
            .saturating_add(cache_write_tokens);

        let friendly_model = obj
            .get("model")
            .and_then(json_nonempty_str)
            .map(ToString::to_string);

        let raw_model_enum = chat_model
            .and_then(|cm| cm.get("model"))
            .and_then(json_nonempty_str)
            .or_else(|| {
                raw_usage
                    .and_then(|u| u.get("model"))
                    .and_then(json_nonempty_str)
            })
            .or_else(|| {
                custom_meta
                    .and_then(|m| m.get("model_enum"))
                    .and_then(json_nonempty_str)
            })
            .map(ToString::to_string);

        let response_model = chat_model
            .and_then(|cm| cm.get("responseModel"))
            .and_then(json_nonempty_str)
            .map(ToString::to_string);

        let raw_api_provider = raw_usage
            .and_then(|u| u.get("apiProvider"))
            .and_then(json_nonempty_str)
            .or_else(|| obj.get("apiProvider").and_then(json_nonempty_str))
            .map(ToString::to_string);

        let message_id = raw_usage
            .and_then(|u| u.get("messageId"))
            .and_then(json_nonempty_str)
            .or_else(|| obj.get("messageId").and_then(json_nonempty_str))
            .map(ToString::to_string);

        let timestamp = obj
            .get("timestamp")
            .or_else(|| obj.get("createdAt"))
            .and_then(json_nonempty_str)
            .and_then(normalize_timestamp);

        by_step.insert(
            step_index,
            CachedRawTurn {
                step_index,
                timestamp,
                friendly_model,
                raw_model_enum,
                response_model,
                raw_api_provider,
                is_exact: true,
                source: DataSourceKind::RpcCache,
                status: 0,
                exact: ExactTokenTotals {
                    input_tokens,
                    output_tokens,
                    thinking_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    total_tokens,
                },
                estimated: TurnEstimatedTotals::default(),
                message_id,
            },
        );
    }

    let mut turns: Vec<CachedRawTurn> = by_step.into_values().collect();
    turns.sort_by_key(|t| t.step_index);
    turns
}

struct ParsedTranscriptTelemetry {
    step_count: usize,
    created_at_ms: Option<i64>,
    turns: Vec<CachedRawTurn>,
}

fn parse_transcript_jsonl_content(content: &str) -> ParsedTranscriptTelemetry {
    let mut by_step: HashMap<usize, CachedRawTurn> = HashMap::new();
    let mut max_step_seen: Option<usize> = None;
    let mut valid_lines = 0usize;
    let mut created_at_ms: Option<i64> = None;
    let last_nonempty_ptr = content
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.as_ptr());

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let is_first_or_last =
            created_at_ms.is_none() || Some(raw_line.as_ptr()) == last_nonempty_ptr;
        let is_planner = line.contains("\"PLANNER_RESPONSE\"");

        if !is_first_or_last && !is_planner {
            if line.starts_with('{') && line.ends_with('}') {
                valid_lines += 1;
            }
            continue;
        }

        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        valid_lines += 1;

        let step_idx = obj
            .get("step_index")
            .and_then(json_u64)
            .map(|n| n as usize)
            .unwrap_or_else(|| valid_lines.saturating_sub(1));

        max_step_seen = Some(max_step_seen.map_or(step_idx, |m| m.max(step_idx)));

        let ts_ms = obj
            .get("created_at")
            .and_then(json_nonempty_str)
            .and_then(parse_timestamp_ms);

        if (step_idx == 0 || created_at_ms.is_none()) && ts_ms.is_some() {
            created_at_ms = ts_ms;
        }

        if obj.get("type").and_then(|v| v.as_str()) != Some("PLANNER_RESPONSE") {
            continue;
        }

        let ts = ts_ms.and_then(format_utc_ms);
        let has_exact_input = obj.get("input_tokens").is_some_and(|v| !v.is_null());
        let has_exact_output = obj.get("output_tokens").is_some_and(|v| !v.is_null());

        let friendly_model = obj
            .get("model")
            .and_then(json_nonempty_str)
            .map(ToString::to_string);

        if has_exact_input || has_exact_output {
            let input_tokens = obj.get("input_tokens").and_then(json_u64).unwrap_or(0);
            let thinking_tokens = obj.get("thinking_tokens").and_then(json_u64).unwrap_or(0);
            let raw_out = obj.get("output_tokens").and_then(json_u64).unwrap_or(0);
            let output_tokens = obj
                .get("response_output_tokens")
                .and_then(json_u64)
                .unwrap_or_else(|| {
                    if thinking_tokens > 0 && raw_out >= thinking_tokens {
                        raw_out - thinking_tokens
                    } else {
                        raw_out
                    }
                });
            let cache_read_tokens = obj.get("cache_read_tokens").and_then(json_u64).unwrap_or(0);
            let cache_write_tokens = obj
                .get("cache_write_tokens")
                .and_then(json_u64)
                .unwrap_or(0);
            let total_tokens = input_tokens
                .saturating_add(output_tokens)
                .saturating_add(thinking_tokens)
                .saturating_add(cache_read_tokens)
                .saturating_add(cache_write_tokens);

            by_step.insert(
                step_idx,
                CachedRawTurn {
                    step_index: step_idx,
                    timestamp: ts,
                    friendly_model,
                    raw_model_enum: None,
                    response_model: None,
                    raw_api_provider: None,
                    is_exact: true,
                    source: DataSourceKind::TranscriptJsonl,
                    status: 0,
                    exact: ExactTokenTotals {
                        input_tokens,
                        output_tokens,
                        thinking_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        total_tokens,
                    },
                    estimated: TurnEstimatedTotals::default(),
                    message_id: None,
                },
            );
        } else {
            let mut readable_bytes = 0usize;
            if let Some(c) = obj.get("content").and_then(|v| v.as_str()) {
                readable_bytes = readable_bytes.saturating_add(c.len());
            }
            if let Some(th) = obj.get("thinking").and_then(|v| v.as_str()) {
                readable_bytes = readable_bytes.saturating_add(th.len());
            }
            if let Some(tc) = obj.get("tool_calls") {
                if !tc.is_null() {
                    readable_bytes = readable_bytes.saturating_add(tc.to_string().len());
                }
            }
            if readable_bytes == 0 {
                readable_bytes = line.len();
            }
            let est_out = estimate_tokens_from_bytes(readable_bytes);
            by_step.insert(
                step_idx,
                CachedRawTurn {
                    step_index: step_idx,
                    timestamp: ts,
                    friendly_model,
                    raw_model_enum: None,
                    response_model: None,
                    raw_api_provider: None,
                    is_exact: false,
                    source: DataSourceKind::TranscriptJsonl,
                    status: 0,
                    exact: ExactTokenTotals::default(),
                    estimated: TurnEstimatedTotals {
                        input_tokens: 0,
                        output_tokens: est_out,
                        total_tokens: est_out,
                    },
                    message_id: None,
                },
            );
        }
    }

    let mut turns: Vec<CachedRawTurn> = by_step.into_values().collect();
    turns.sort_by_key(|t| t.step_index);
    let step_count = max_step_seen
        .map(|m| m.saturating_add(1))
        .unwrap_or(0)
        .max(valid_lines);

    ParsedTranscriptTelemetry {
        step_count,
        created_at_ms,
        turns,
    }
}

fn open_sqlite_readonly(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(path, READ_ONLY_SQLITE_FLAGS)?;
    let _ = conn.busy_timeout(Duration::from_millis(250));
    let _ = conn.execute_batch(
        "PRAGMA query_only = ON; PRAGMA temp_store = MEMORY; PRAGMA mmap_size = 67108864;",
    );
    Ok(conn)
}

fn value_ref_to_string(val: ValueRef<'_>) -> String {
    match val {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
        ValueRef::Blob(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

fn value_ref_to_usize(val: ValueRef<'_>) -> usize {
    match val {
        ValueRef::Integer(i) if i > 0 => i as usize,
        ValueRef::Text(b) => std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

fn read_conversation_summaries_db(db_path: &Path) -> Result<HashMap<String, SummaryDbRow>, String> {
    let conn = open_sqlite_readonly(db_path).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();

    let full_sql = "SELECT conversation_id, title, preview, step_count, last_modified_time, \
                    workspace_uris, status, project_id, agent_name, parent_conversation_id, nesting_depth \
                    FROM conversation_summaries";

    if let Ok(mut stmt) = conn.prepare(full_sql) {
        let rows = stmt
            .query_map([], |row| {
                Ok(SummaryDbRow {
                    conversation_id: value_ref_to_string(row.get_ref(0)?).trim().to_string(),
                    title: value_ref_to_string(row.get_ref(1)?),
                    preview: value_ref_to_string(row.get_ref(2)?),
                    step_count: value_ref_to_usize(row.get_ref(3)?),
                    last_modified_ms: parse_timestamp_ms(&value_ref_to_string(row.get_ref(4)?)),
                    workspace_uris: value_ref_to_string(row.get_ref(5)?),
                    status: value_ref_to_string(row.get_ref(6)?),
                    project_id: value_ref_to_string(row.get_ref(7)?),
                    agent_name: value_ref_to_string(row.get_ref(8)?).trim().to_string(),
                    parent_conversation_id: value_ref_to_string(row.get_ref(9)?).trim().to_string(),
                    nesting_depth: value_ref_to_usize(row.get_ref(10)?),
                })
            })
            .map_err(|e| e.to_string())?;

        for row_res in rows {
            if let Ok(r) = row_res {
                if !r.conversation_id.is_empty() {
                    out.insert(r.conversation_id.clone(), r);
                }
            }
        }
        return Ok(out);
    }

    let minimal_sql =
        "SELECT conversation_id, title, preview, step_count, last_modified_time, workspace_uris \
                       FROM conversation_summaries";
    let mut stmt = conn.prepare(minimal_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SummaryDbRow {
                conversation_id: value_ref_to_string(row.get_ref(0)?).trim().to_string(),
                title: value_ref_to_string(row.get_ref(1)?),
                preview: value_ref_to_string(row.get_ref(2)?),
                step_count: value_ref_to_usize(row.get_ref(3)?),
                last_modified_ms: parse_timestamp_ms(&value_ref_to_string(row.get_ref(4)?)),
                workspace_uris: value_ref_to_string(row.get_ref(5)?),
                ..Default::default()
            })
        })
        .map_err(|e| e.to_string())?;

    for row_res in rows {
        if let Ok(r) = row_res {
            if !r.conversation_id.is_empty() {
                out.insert(r.conversation_id.clone(), r);
            }
        }
    }

    Ok(out)
}

#[derive(Default)]
struct GroupAggregator {
    map: HashMap<(Option<i64>, RawModelKey), CachedTurnGroup>,
}

impl GroupAggregator {
    fn push_exact(&mut self, ts_ms: Option<i64>, model_key: RawModelKey, exact: ExactTokenTotals) {
        let minute_slot = ts_ms.map(|ms| ms.div_euclid(60_000));
        let entry = self
            .map
            .entry((minute_slot, model_key.clone()))
            .or_insert_with(|| CachedTurnGroup {
                max_ts_ms: ts_ms,
                model_key,
                exact_turn_count: 0,
                estimated_turn_count: 0,
                exact: ExactTokenTotals::default(),
                estimated: TurnEstimatedTotals::default(),
            });
        if let Some(ms) = ts_ms {
            entry.max_ts_ms = Some(entry.max_ts_ms.map_or(ms, |prev| prev.max(ms)));
        }
        entry.exact_turn_count += 1;
        entry.exact.add_assign(&exact);
    }

    fn push_estimated(
        &mut self,
        ts_ms: Option<i64>,
        model_key: RawModelKey,
        estimated: TurnEstimatedTotals,
    ) {
        let minute_slot = ts_ms.map(|ms| ms.div_euclid(60_000));
        let entry = self
            .map
            .entry((minute_slot, model_key.clone()))
            .or_insert_with(|| CachedTurnGroup {
                max_ts_ms: ts_ms,
                model_key,
                exact_turn_count: 0,
                estimated_turn_count: 0,
                exact: ExactTokenTotals::default(),
                estimated: TurnEstimatedTotals::default(),
            });
        if let Some(ms) = ts_ms {
            entry.max_ts_ms = Some(entry.max_ts_ms.map_or(ms, |prev| prev.max(ms)));
        }
        entry.estimated_turn_count += 1;
        entry.estimated.input_tokens = entry
            .estimated
            .input_tokens
            .saturating_add(estimated.input_tokens);
        entry.estimated.output_tokens = entry
            .estimated
            .output_tokens
            .saturating_add(estimated.output_tokens);
        entry.estimated.total_tokens = entry
            .estimated
            .total_tokens
            .saturating_add(estimated.total_tokens);
    }

    fn from_raw_turns(raw_turns: &[CachedRawTurn]) -> Vec<CachedTurnGroup> {
        let mut agg = Self::default();
        for rt in raw_turns {
            let ts_ms = rt.timestamp.as_deref().and_then(parse_timestamp_ms);
            let key = RawModelKey {
                friendly_model: rt.friendly_model.clone(),
                raw_model_enum: rt.raw_model_enum.clone(),
                response_model: rt.response_model.clone(),
                raw_api_provider: rt.raw_api_provider.clone(),
            };
            if rt.is_exact {
                agg.push_exact(ts_ms, key, rt.exact);
            } else {
                agg.push_estimated(ts_ms, key, rt.estimated);
            }
        }
        agg.into_sorted_vec()
    }

    fn into_sorted_vec(self) -> Vec<CachedTurnGroup> {
        let mut out: Vec<CachedTurnGroup> = self.map.into_values().collect();
        out.sort_by(|a, b| {
            a.max_ts_ms
                .cmp(&b.max_ts_ms)
                .then_with(|| a.model_key.raw_model_enum.cmp(&b.model_key.raw_model_enum))
                .then_with(|| a.model_key.response_model.cmp(&b.model_key.response_model))
                .then_with(|| a.model_key.friendly_model.cmp(&b.model_key.friendly_model))
        });
        out
    }
}

#[inline]
fn u64_enum_to_string(v: u64) -> String {
    match v {
        1318 => "1318".to_string(),
        1322 => "1322".to_string(),
        1401 => "1401".to_string(),
        1402 => "1402".to_string(),
        1417 => "1417".to_string(),
        1427 => "1427".to_string(),
        24 => "24".to_string(),
        26 => "26".to_string(),
        other => other.to_string(),
    }
}

/// Ultra-fast zero-copy SQLite extraction producing minute-bucketed `CachedTurnGroup` rollups.
fn extract_from_sqlite_db_groups(
    db_path: &Path,
    rpc_usage_path: Option<&Path>,
    transcript_path: Option<&Path>,
) -> Result<(usize, Option<i64>, bool, Vec<CachedTurnGroup>), String> {
    let conn = open_sqlite_readonly(db_path).map_err(|e| e.to_string())?;

    let is_native_swarm_worker = conn
        .query_row(
            "SELECT instr(data, X'4e617469766520737761726d20776f726b6572') > 0 \
             FROM trajectory_metadata_blob WHERE id = 'main'",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    // O(log N) primary-key rightmost seek instead of full table scan `COUNT(*)`.
    let step_count: usize = conn
        .query_row("SELECT IFNULL(MAX(idx) + 1, 0) FROM steps", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|c| c.max(0) as usize)
        .map_err(|e| e.to_string())?;

    let mut created_at_ms: Option<i64> = conn
        .query_row("SELECT metadata FROM steps WHERE idx = 0", [], |row| {
            let meta_ts = match row.get_ref(0)? {
                ValueRef::Blob(b) if !b.is_empty() => parse_step_metadata_slim(b).timestamp_ms,
                _ => None,
            };
            Ok(meta_ts)
        })
        .ok()
        .flatten();

    // Only select columns up to `metadata` (col 4) using `idx_steps_step_type`,
    // never touching overflow pages for `render_info` or `step_payload`.
    let mut planner_stmt = conn
        .prepare("SELECT idx, metadata FROM steps WHERE step_type = 15")
        .map_err(|e| e.to_string())?;

    let mut step_rows: Vec<(usize, SlimStepMetadataProto)> = Vec::new();
    {
        let rows = planner_stmt
            .query_map([], |row| {
                let idx = row.get::<_, i64>(0)?.max(0) as usize;
                let slim = match row.get_ref(1)? {
                    ValueRef::Blob(b) if !b.is_empty() => parse_step_metadata_slim(b),
                    _ => SlimStepMetadataProto::default(),
                };
                Ok((idx, slim))
            })
            .map_err(|e| e.to_string())?;

        for row_res in rows {
            step_rows.push(row_res.map_err(|e| e.to_string())?);
        }
    }

    if step_rows.is_empty() {
        return Ok((
            step_count,
            created_at_ms,
            is_native_swarm_worker,
            Vec::new(),
        ));
    }

    let need_gen_meta = step_rows.iter().any(|(_, sm)| match &sm.usage {
        None => true,
        Some(u) => !u.model_enum.is_some_and(is_builtin_enum_u64),
    });
    let need_gen_usage = step_rows.iter().any(|(_, sm)| sm.usage.is_none());

    let mut gen_by_step: HashMap<usize, SlimGenMetadataProto> = HashMap::new();
    if need_gen_meta {
        if let Ok(mut gen_stmt) = conn.prepare("SELECT data FROM gen_metadata") {
            if let Ok(rows) = gen_stmt.query_map([], |row| {
                let parsed = match row.get_ref(0)? {
                    ValueRef::Blob(b) if !b.is_empty() => {
                        Some(parse_gen_metadata_slim(b, need_gen_usage))
                    }
                    _ => None,
                };
                Ok(parsed)
            }) {
                for parsed in rows.flatten().flatten() {
                    if parsed.step_indices.len() == 1 {
                        let s_idx = parsed.step_indices[0];
                        gen_by_step.insert(s_idx, parsed);
                    } else {
                        for &s_idx in &parsed.step_indices {
                            gen_by_step.insert(s_idx, parsed.clone());
                        }
                    }
                }
            }
        }
    }

    let mut payload_stmt: Option<rusqlite::Statement<'_>> = None;
    let mut lazy_rpc_by_step: Option<HashMap<usize, CachedRawTurn>> = None;
    let mut lazy_transcript_by_step: Option<HashMap<usize, CachedRawTurn>> = None;
    let mut groups = GroupAggregator::default();

    for (idx, step_meta) in step_rows {
        if created_at_ms.is_none() && step_meta.timestamp_ms.is_some() {
            created_at_ms = step_meta.timestamp_ms;
        }

        let gen_meta = if need_gen_meta {
            gen_by_step.get(&idx)
        } else {
            None
        };

        let proto_usage = step_meta
            .usage
            .as_ref()
            .or_else(|| gen_meta.and_then(|g| g.usage.as_ref()));

        if let Some(usage) = proto_usage {
            let raw_model_enum = usage
                .model_enum
                .map(u64_enum_to_string)
                .or_else(|| gen_meta.and_then(|g| g.model_enum_placeholder.clone()))
                .or_else(|| {
                    gen_meta
                        .and_then(|g| g.model_enum_numeric)
                        .map(u64_enum_to_string)
                })
                .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));

            let response_model = gen_meta.and_then(|g| g.response_model.clone());
            let raw_api_provider = usage.api_provider.map(u64_enum_to_string);

            groups.push_exact(
                step_meta.timestamp_ms,
                RawModelKey {
                    friendly_model: None,
                    raw_model_enum,
                    response_model,
                    raw_api_provider,
                },
                ExactTokenTotals {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    thinking_tokens: usage.thinking_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                    cache_write_tokens: usage.cache_write_tokens,
                    total_tokens: usage.total_tokens,
                },
            );
            continue;
        }

        if lazy_rpc_by_step.is_none() {
            let mut map = HashMap::new();
            if let Some(p) = rpc_usage_path {
                if let Ok(content) = std::fs::read_to_string(p) {
                    for t in parse_rpc_usage_jsonl_content(&content) {
                        map.insert(t.step_index, t);
                    }
                }
            }
            lazy_rpc_by_step = Some(map);
        }
        if let Some(rpc_turn) = lazy_rpc_by_step.as_ref().and_then(|m| m.get(&idx)) {
            let ts_ms = rpc_turn
                .timestamp
                .as_deref()
                .and_then(parse_timestamp_ms)
                .or(step_meta.timestamp_ms);
            groups.push_exact(
                ts_ms,
                RawModelKey {
                    friendly_model: rpc_turn.friendly_model.clone(),
                    raw_model_enum: rpc_turn.raw_model_enum.clone(),
                    response_model: rpc_turn.response_model.clone(),
                    raw_api_provider: rpc_turn.raw_api_provider.clone(),
                },
                rpc_turn.exact,
            );
            continue;
        }

        if lazy_transcript_by_step.is_none() {
            let mut map = HashMap::new();
            if let Some(p) = transcript_path {
                if let Ok(content) = std::fs::read_to_string(p) {
                    for t in parse_transcript_jsonl_content(&content).turns {
                        if t.is_exact {
                            map.insert(t.step_index, t);
                        }
                    }
                }
            }
            lazy_transcript_by_step = Some(map);
        }
        if let Some(tr_turn) = lazy_transcript_by_step.as_ref().and_then(|m| m.get(&idx)) {
            let ts_ms = tr_turn
                .timestamp
                .as_deref()
                .and_then(parse_timestamp_ms)
                .or(step_meta.timestamp_ms);
            let raw_model_enum = tr_turn
                .raw_model_enum
                .clone()
                .or_else(|| gen_meta.and_then(|g| g.model_enum_placeholder.clone()))
                .or_else(|| {
                    gen_meta
                        .and_then(|g| g.model_enum_numeric)
                        .map(u64_enum_to_string)
                })
                .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));
            let response_model = tr_turn
                .response_model
                .clone()
                .or_else(|| gen_meta.and_then(|g| g.response_model.clone()));
            groups.push_exact(
                ts_ms,
                RawModelKey {
                    friendly_model: tr_turn.friendly_model.clone(),
                    raw_model_enum,
                    response_model,
                    raw_api_provider: tr_turn.raw_api_provider.clone(),
                },
                tr_turn.exact,
            );
            continue;
        }

        if payload_stmt.is_none() {
            payload_stmt = conn
                .prepare("SELECT LENGTH(step_payload), step_payload FROM steps WHERE idx = ?1")
                .ok();
        }
        let (payload_len, payload_bytes): (usize, Vec<u8>) = payload_stmt
            .as_mut()
            .and_then(|st| {
                st.query_row([idx as i64], |r| {
                    let len = r.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as usize;
                    let bytes = r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default();
                    Ok((len, bytes))
                })
                .ok()
            })
            .unwrap_or((0, Vec::new()));

        let readable = extract_readable_utf8_bytes(&payload_bytes, payload_len);
        let est_out = estimate_tokens_from_bytes(readable);

        let raw_model_enum = gen_meta
            .and_then(|g| g.model_enum_placeholder.clone())
            .or_else(|| {
                gen_meta
                    .and_then(|g| g.model_enum_numeric)
                    .map(u64_enum_to_string)
            })
            .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));

        groups.push_estimated(
            step_meta.timestamp_ms,
            RawModelKey {
                friendly_model: None,
                raw_model_enum,
                response_model: gen_meta.and_then(|g| g.response_model.clone()),
                raw_api_provider: None,
            },
            TurnEstimatedTotals {
                input_tokens: 0,
                output_tokens: est_out,
                total_tokens: est_out,
            },
        );
    }

    Ok((
        step_count,
        created_at_ms,
        is_native_swarm_worker,
        groups.into_sorted_vec(),
    ))
}

/// On-demand full turn extraction for a SINGLE conversation (called only when opening the detail modal).
fn extract_from_sqlite_db_full_turns(
    db_path: &Path,
    rpc_usage_path: Option<&Path>,
    transcript_path: Option<&Path>,
) -> Result<Vec<CachedRawTurn>, String> {
    let conn = open_sqlite_readonly(db_path).map_err(|e| e.to_string())?;

    let mut gen_by_step: HashMap<usize, ParsedGenMetadataProto> = HashMap::new();
    if let Ok(mut gen_stmt) = conn.prepare("SELECT data FROM gen_metadata") {
        if let Ok(rows) = gen_stmt.query_map([], |row| {
            let parsed = match row.get_ref(0)? {
                ValueRef::Blob(b) if !b.is_empty() => Some(parse_gen_metadata_proto(b)),
                _ => None,
            };
            Ok(parsed)
        }) {
            for parsed in rows.flatten().flatten() {
                for &s_idx in &parsed.step_indices {
                    gen_by_step.insert(s_idx, parsed.clone());
                }
            }
        }
    }

    let mut planner_stmt = conn
        .prepare("SELECT idx, status, metadata FROM steps WHERE step_type = 15 ORDER BY idx ASC")
        .map_err(|e| e.to_string())?;
    let mut payload_stmt: Option<rusqlite::Statement<'_>> = None;
    let mut lazy_rpc_by_step: Option<HashMap<usize, CachedRawTurn>> = None;
    let mut lazy_transcript_by_step: Option<HashMap<usize, CachedRawTurn>> = None;

    let rows = planner_stmt
        .query_map([], |row| {
            let idx = row.get::<_, i64>(0)?.max(0) as usize;
            let status = row.get::<_, Option<i32>>(1)?.unwrap_or(0);
            let step_meta = match row.get_ref(2)? {
                ValueRef::Blob(b) if !b.is_empty() => parse_step_metadata_proto(b),
                _ => Default::default(),
            };
            Ok((idx, status, step_meta))
        })
        .map_err(|e| e.to_string())?;

    let mut turns = Vec::new();
    for row_res in rows {
        let (idx, status, step_meta) = row_res.map_err(|e| e.to_string())?;
        let gen_meta = gen_by_step.get(&idx);

        let proto_usage = step_meta
            .usage
            .as_ref()
            .or_else(|| gen_meta.and_then(|g| g.usage.as_ref()));

        if let Some(usage) = proto_usage {
            let raw_model_enum = usage
                .model_enum
                .map(u64_enum_to_string)
                .or_else(|| gen_meta.and_then(|g| g.model_enum_placeholder.clone()))
                .or_else(|| {
                    gen_meta
                        .and_then(|g| g.model_enum_numeric)
                        .map(u64_enum_to_string)
                })
                .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));

            turns.push(CachedRawTurn {
                step_index: idx,
                timestamp: step_meta.timestamp,
                friendly_model: None,
                raw_model_enum,
                response_model: gen_meta.and_then(|g| g.response_model.clone()),
                raw_api_provider: usage.api_provider.map(u64_enum_to_string),
                is_exact: true,
                source: DataSourceKind::SqliteDb,
                status,
                exact: ExactTokenTotals {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    thinking_tokens: usage.thinking_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                    cache_write_tokens: usage.cache_write_tokens,
                    total_tokens: usage.total_tokens,
                },
                estimated: TurnEstimatedTotals::default(),
                message_id: usage.message_id.clone(),
            });
            continue;
        }

        if lazy_rpc_by_step.is_none() {
            let mut map = HashMap::new();
            if let Some(p) = rpc_usage_path {
                if let Ok(content) = std::fs::read_to_string(p) {
                    for t in parse_rpc_usage_jsonl_content(&content) {
                        map.insert(t.step_index, t);
                    }
                }
            }
            lazy_rpc_by_step = Some(map);
        }
        if let Some(mut rpc_turn) = lazy_rpc_by_step.as_ref().and_then(|m| m.get(&idx)).cloned() {
            if rpc_turn.timestamp.is_none() {
                rpc_turn.timestamp = step_meta.timestamp;
            }
            rpc_turn.status = status;
            turns.push(rpc_turn);
            continue;
        }

        if lazy_transcript_by_step.is_none() {
            let mut map = HashMap::new();
            if let Some(p) = transcript_path {
                if let Ok(content) = std::fs::read_to_string(p) {
                    for t in parse_transcript_jsonl_content(&content).turns {
                        if t.is_exact {
                            map.insert(t.step_index, t);
                        }
                    }
                }
            }
            lazy_transcript_by_step = Some(map);
        }
        if let Some(mut tr_turn) = lazy_transcript_by_step
            .as_ref()
            .and_then(|m| m.get(&idx))
            .cloned()
        {
            if tr_turn.timestamp.is_none() {
                tr_turn.timestamp = step_meta.timestamp;
            }
            if tr_turn.raw_model_enum.is_none() {
                tr_turn.raw_model_enum = gen_meta
                    .and_then(|g| g.model_enum_placeholder.clone())
                    .or_else(|| {
                        gen_meta
                            .and_then(|g| g.model_enum_numeric)
                            .map(u64_enum_to_string)
                    })
                    .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));
            }
            if tr_turn.response_model.is_none() {
                tr_turn.response_model = gen_meta.and_then(|g| g.response_model.clone());
            }
            tr_turn.status = status;
            turns.push(tr_turn);
            continue;
        }

        if payload_stmt.is_none() {
            payload_stmt = conn
                .prepare("SELECT LENGTH(step_payload), step_payload FROM steps WHERE idx = ?1")
                .ok();
        }
        let (payload_len, payload_bytes): (usize, Vec<u8>) = payload_stmt
            .as_mut()
            .and_then(|st| {
                st.query_row([idx as i64], |r| {
                    let len = r.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as usize;
                    let bytes = r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default();
                    Ok((len, bytes))
                })
                .ok()
            })
            .unwrap_or((0, Vec::new()));

        let readable = extract_readable_utf8_bytes(&payload_bytes, payload_len);
        let est_out = estimate_tokens_from_bytes(readable);

        let raw_model_enum = gen_meta
            .and_then(|g| g.model_enum_placeholder.clone())
            .or_else(|| {
                gen_meta
                    .and_then(|g| g.model_enum_numeric)
                    .map(u64_enum_to_string)
            })
            .or_else(|| step_meta.fallback_model_enum.map(u64_enum_to_string));

        turns.push(CachedRawTurn {
            step_index: idx,
            timestamp: step_meta.timestamp,
            friendly_model: None,
            raw_model_enum,
            response_model: gen_meta.and_then(|g| g.response_model.clone()),
            raw_api_provider: None,
            is_exact: false,
            source: DataSourceKind::SqliteDb,
            status,
            exact: ExactTokenTotals::default(),
            estimated: TurnEstimatedTotals {
                input_tokens: 0,
                output_tokens: est_out,
                total_tokens: est_out,
            },
            message_id: None,
        });
    }

    Ok(turns)
}

/// Builds a single `ConversationDetail` on demand in ~1–2 ms when the user opens a session modal.
pub fn build_conversation_detail(
    scan: &ScanResult,
    conversation_id: &str,
) -> Result<ConversationDetail, String> {
    let idx = scan
        .index_by_id
        .get(conversation_id)
        .copied()
        .ok_or_else(|| format!("Conversation not found: {conversation_id}"))?;
    let conv = &scan.conversations[idx];

    let db_path = scan
        .data_dir
        .join("conversations")
        .join(format!("{conversation_id}.db"));
    let rpc_path = scan
        .data_dir
        .join(".token-monitor")
        .join("rpc-cache")
        .join("v1")
        .join(conversation_id)
        .join("usage.jsonl");
    let tr_path = scan
        .data_dir
        .join("brain")
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");

    let has_rpc = rpc_path.is_file();
    let has_tr = tr_path.is_file();

    let raw_turns: Vec<CachedRawTurn> = if db_path.is_file() {
        extract_from_sqlite_db_full_turns(
            &db_path,
            has_rpc.then_some(rpc_path.as_path()),
            has_tr.then_some(tr_path.as_path()),
        )
        .unwrap_or_default()
    } else if has_rpc {
        std::fs::read_to_string(&rpc_path)
            .map(|c| parse_rpc_usage_jsonl_content(&c))
            .unwrap_or_default()
    } else if has_tr {
        std::fs::read_to_string(&tr_path)
            .map(|c| parse_transcript_jsonl_content(&c).turns)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut turns = Vec::with_capacity(raw_turns.len());
    for rt in raw_turns {
        let mut resolved = resolve_model_info(ModelResolutionInput {
            friendly_model: rt.friendly_model.as_deref(),
            raw_model_enum: rt.raw_model_enum.as_deref(),
            response_model: rt.response_model.as_deref(),
            api_provider: rt.raw_api_provider.as_deref(),
            dynamic_enum_map: Some(&scan.dynamic_enum_map),
        });
        if resolved.model_id == "unknown" {
            if let Some(hint) = conv.model_hint.as_deref() {
                resolved = resolve_model_info(ModelResolutionInput {
                    friendly_model: Some(hint),
                    api_provider: rt.raw_api_provider.as_deref(),
                    dynamic_enum_map: Some(&scan.dynamic_enum_map),
                    ..Default::default()
                });
            }
        }
        turns.push(TurnUsageRecord {
            step_index: rt.step_index,
            timestamp: rt.timestamp,
            model_id: resolved.model_id,
            model_display_name: resolved.display_name,
            raw_model_enum: rt.raw_model_enum,
            response_model: rt.response_model,
            api_provider: resolved.api_provider,
            is_exact: rt.is_exact,
            source: rt.source,
            status: rt.status,
            exact: rt.exact,
            estimated: rt.estimated,
            message_id: rt.message_id,
        });
    }

    Ok(ConversationDetail {
        summary: conv.summary.clone(),
        model_breakdown: conv.model_breakdown.clone(),
        turns,
        children: conv.children.clone(),
    })
}

fn extract_conversation_telemetry(task: &ExtractionTask) -> ExtractionOutcome {
    let mut warning: Option<String> = None;

    if let Some(ref db_path) = task.db_path {
        match extract_from_sqlite_db_groups(
            db_path,
            task.rpc_usage_path.as_deref(),
            task.transcript_path.as_deref(),
        ) {
            Ok((db_step_count, created_at_ms, is_native_swarm_worker, groups)) => {
                return ExtractionOutcome {
                    conversation_id: task.conversation_id.clone(),
                    telemetry: CachedConversationTelemetry {
                        fingerprint: task.fingerprint,
                        data_source: DataSourceKind::SqliteDb,
                        db_step_count,
                        created_at_ms,
                        is_native_swarm_worker,
                        groups,
                    },
                    warning: None,
                };
            }
            Err(err) => {
                warning = Some(format!("{}: {}", task.conversation_id, err));
            }
        }
    }

    if let Some(ref rpc_path) = task.rpc_usage_path {
        if let Ok(content) = std::fs::read_to_string(rpc_path) {
            let turns = parse_rpc_usage_jsonl_content(&content);
            let created_at_ms = turns
                .iter()
                .find_map(|t| t.timestamp.as_deref().and_then(parse_timestamp_ms));
            let step_count = turns
                .last()
                .map(|t| t.step_index.saturating_add(1))
                .unwrap_or(0)
                .max(turns.len());
            let groups = GroupAggregator::from_raw_turns(&turns);
            return ExtractionOutcome {
                conversation_id: task.conversation_id.clone(),
                telemetry: CachedConversationTelemetry {
                    fingerprint: task.fingerprint,
                    data_source: DataSourceKind::RpcCache,
                    db_step_count: step_count,
                    created_at_ms,
                    is_native_swarm_worker: false,
                    groups,
                },
                warning,
            };
        }
    }

    if let Some(ref tr_path) = task.transcript_path {
        if let Ok(content) = std::fs::read_to_string(tr_path) {
            let parsed = parse_transcript_jsonl_content(&content);
            let groups = GroupAggregator::from_raw_turns(&parsed.turns);
            return ExtractionOutcome {
                conversation_id: task.conversation_id.clone(),
                telemetry: CachedConversationTelemetry {
                    fingerprint: task.fingerprint,
                    data_source: DataSourceKind::TranscriptJsonl,
                    db_step_count: parsed.step_count,
                    created_at_ms: parsed.created_at_ms,
                    is_native_swarm_worker: false,
                    groups,
                },
                warning,
            };
        }
    }

    ExtractionOutcome {
        conversation_id: task.conversation_id.clone(),
        telemetry: CachedConversationTelemetry {
            fingerprint: task.fingerprint,
            data_source: DataSourceKind::SummaryOnly,
            db_step_count: 0,
            created_at_ms: None,
            is_native_swarm_worker: false,
            groups: Vec::new(),
        },
        warning,
    }
}

/// Largest-Processing-Time (LPT) work-stealing parallel extractor.
fn parallel_extract(tasks: &[ExtractionTask]) -> Vec<ExtractionOutcome> {
    if tasks.is_empty() {
        return Vec::new();
    }
    if tasks.len() == 1 {
        return vec![extract_conversation_telemetry(&tasks[0])];
    }

    let mut ordered_refs: Vec<&ExtractionTask> = tasks.iter().collect();
    ordered_refs.sort_unstable_by(|a, b| {
        b.fingerprint
            .weight_bytes()
            .cmp(&a.fingerprint.weight_bytes())
    });

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
        .min(ordered_refs.len());

    let cursor = AtomicUsize::new(0);
    let ordered_slice = ordered_refs.as_slice();

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let cursor_ref = &cursor;
            handles.push(scope.spawn(move || {
                let mut local_out = Vec::new();
                loop {
                    let idx = cursor_ref.fetch_add(1, AtomicOrdering::Relaxed);
                    if idx >= ordered_slice.len() {
                        break;
                    }
                    local_out.push(extract_conversation_telemetry(ordered_slice[idx]));
                }
                local_out
            }));
        }

        let mut out = Vec::with_capacity(tasks.len());
        for handle in handles {
            if let Ok(part) = handle.join() {
                out.extend(part);
            }
        }
        out
    })
}

fn parse_swarm_members_file(
    brain_conv_id: &str,
    members_path: &Path,
) -> (Option<String>, Vec<(String, SwarmWorkerMeta)>) {
    let Ok(content) = std::fs::read_to_string(members_path) else {
        return (None, Vec::new());
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return (None, Vec::new());
    };
    let Some(members_val) = root.get("members") else {
        return (None, Vec::new());
    };

    let mut entries: Vec<(Option<String>, &serde_json::Value)> = Vec::new();
    if let Some(map) = members_val.as_object() {
        for (k, v) in map {
            entries.push((Some(k.clone()), v));
        }
    } else if let Some(arr) = members_val.as_array() {
        for v in arr {
            entries.push((None, v));
        }
    } else {
        return (None, Vec::new());
    }

    let mut coordinator_id = brain_conv_id.to_string();
    for (_, member) in &entries {
        let role = member
            .get("role")
            .and_then(json_nonempty_str)
            .unwrap_or("worker");
        if role.eq_ignore_ascii_case("coordinator") {
            if let Some(cid) = member.get("conversation_id").and_then(json_nonempty_str) {
                coordinator_id = cid.to_string();
                break;
            }
        }
    }

    let mut workers = Vec::new();
    for (obj_key, member) in entries {
        let role = member
            .get("role")
            .and_then(json_nonempty_str)
            .unwrap_or("worker");
        if role.eq_ignore_ascii_case("coordinator") {
            continue;
        }
        let Some(worker_conv_id) = member.get("conversation_id").and_then(json_nonempty_str) else {
            continue;
        };

        let agent_name = member
            .get("name")
            .and_then(json_nonempty_str)
            .or_else(|| member.get("agent_name").and_then(json_nonempty_str))
            .map(ToString::to_string)
            .or(obj_key)
            .unwrap_or_default();

        let model_hint = member
            .get("applied_model")
            .and_then(json_nonempty_str)
            .or_else(|| member.get("model").and_then(json_nonempty_str))
            .or_else(|| member.get("desired_model").and_then(json_nonempty_str))
            .map(ToString::to_string);

        workers.push((
            worker_conv_id.to_string(),
            SwarmWorkerMeta {
                coordinator_id: coordinator_id.clone(),
                agent_name,
                model_hint,
            },
        ));
    }

    let coord_opt = (!workers.is_empty()).then_some(coordinator_id);
    (coord_opt, workers)
}

fn read_rpc_session_miss(
    dir_path: &Path,
    has_manifest: bool,
    has_usage: bool,
    u_size: u64,
    u_mtime: u64,
) -> RpcSessionMeta {
    let mut meta = RpcSessionMeta {
        has_usage,
        usage_size: u_size,
        usage_mtime_ms: u_mtime,
        ..Default::default()
    };

    if has_manifest {
        let manifest_path = dir_path.join("manifest.json");
        if let Ok(m_text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(m_json) = serde_json::from_str::<serde_json::Value>(&m_text) {
                meta.title = m_json
                    .get("title")
                    .and_then(json_nonempty_str)
                    .map(ToString::to_string);
                meta.step_count = m_json
                    .get("stepCount")
                    .and_then(json_u64)
                    .map(|n| n as usize)
                    .unwrap_or(0);
                meta.last_modified_ms = m_json
                    .get("serverLastModifiedMs")
                    .and_then(json_u64)
                    .and_then(|ms| i64::try_from(ms).ok())
                    .filter(|&ms| ms > 0);
            }
        }
    }

    if u_size > 0 {
        let usage_path = dir_path.join("usage.jsonl");
        if let Ok(f) = File::open(&usage_path) {
            let cap = (u_size.min(RPC_HARVEST_HEAD_BYTES)) as usize;
            let mut head = Vec::with_capacity(cap);
            if f.take(RPC_HARVEST_HEAD_BYTES)
                .read_to_end(&mut head)
                .is_ok()
            {
                let text = String::from_utf8_lossy(&head);
                let mut seen_pairs = HashSet::new();
                for raw_line in text.lines() {
                    let line = raw_line.trim();
                    if line.is_empty() || !line.contains("\"model\"") {
                        continue;
                    }
                    let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
                        continue;
                    };
                    let Some(friendly) = obj.get("model").and_then(json_nonempty_str) else {
                        continue;
                    };
                    if friendly
                        .to_ascii_uppercase()
                        .starts_with("MODEL_PLACEHOLDER_")
                    {
                        continue;
                    }
                    let canonical = canonicalize_model_id(friendly);
                    let cm = obj.get("raw").and_then(|r| r.get("chatModel"));
                    let candidates = [
                        cm.and_then(|c| c.get("model")).and_then(json_nonempty_str),
                        cm.and_then(|c| c.get("usage"))
                            .and_then(|u| u.get("model"))
                            .and_then(json_nonempty_str),
                        cm.and_then(|c| c.get("customMetadata"))
                            .and_then(|m| m.get("model_enum"))
                            .and_then(json_nonempty_str),
                    ];
                    for raw_key in candidates.into_iter().flatten() {
                        if seen_pairs.insert((raw_key.to_string(), canonical.clone())) {
                            meta.enums.push((raw_key.to_string(), canonical.clone()));
                        }
                    }
                }
            }
        }
    }

    meta
}

pub struct AntigravityScanner {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    persist_cache: bool,
    cache_loaded_from_disk: bool,
    telemetry_cache: HashMap<String, CachedConversationTelemetry>,
    dynamic_enum_map: HashMap<String, String>,
    annotation_cache: HashMap<String, AnnotationCacheEntry>,
    rpc_meta_cache: HashMap<String, RpcMetaCacheEntry>,
    swarm_dir_cache: HashMap<String, SwarmDirCacheEntry>,
}

impl AntigravityScanner {
    pub fn new(data_dir: Option<PathBuf>, cache_dir: PathBuf, persist_cache: bool) -> Self {
        let resolved = resolve_antigravity_data_dir(data_dir.as_deref());
        Self {
            data_dir: resolved,
            cache_dir,
            persist_cache,
            cache_loaded_from_disk: false,
            telemetry_cache: HashMap::new(),
            dynamic_enum_map: HashMap::new(),
            annotation_cache: HashMap::new(),
            rpc_meta_cache: HashMap::new(),
            swarm_dir_cache: HashMap::new(),
        }
    }

    fn data_dir_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.data_dir.to_string_lossy().hash(&mut hasher);
        hasher.finish()
    }

    pub fn cache_file_path(&self) -> PathBuf {
        let dir_hash = self.data_dir_hash();
        self.cache_dir
            .join(format!("scan-cache-v3-{dir_hash:016x}.json"))
    }

    fn legacy_v2_cache_file_path(&self) -> PathBuf {
        let dir_hash = self.data_dir_hash();
        self.cache_dir
            .join(format!("scan-cache-v2-{dir_hash:016x}.json"))
    }

    fn ensure_disk_cache_loaded(&mut self) {
        if self.cache_loaded_from_disk || !self.persist_cache {
            return;
        }
        self.cache_loaded_from_disk = true;

        let path = self.cache_file_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(parsed) = serde_json::from_str::<PersistedScanCacheFile>(&content) else {
            return;
        };
        if parsed.schema_version != CACHE_SCHEMA_VERSION {
            return;
        }

        let pool = &parsed.string_pool;
        let mut loaded_entries = HashMap::with_capacity(parsed.entries.len());
        for (conv_id, p_entry) in parsed.entries {
            let mut groups = Vec::with_capacity(p_entry.g.len());
            for row in p_entry.g {
                let max_ts_ms = (row.0 != NO_TIMESTAMP_SENTINEL).then_some(row.0);
                let exact = ExactTokenTotals {
                    input_tokens: row.7,
                    output_tokens: row.8,
                    thinking_tokens: row.9,
                    cache_read_tokens: row.10,
                    cache_write_tokens: row.11,
                    total_tokens: row
                        .7
                        .saturating_add(row.8)
                        .saturating_add(row.9)
                        .saturating_add(row.10)
                        .saturating_add(row.11),
                };
                let estimated = TurnEstimatedTotals {
                    input_tokens: row.12,
                    output_tokens: row.13,
                    total_tokens: row.12.saturating_add(row.13),
                };
                groups.push(CachedTurnGroup {
                    max_ts_ms,
                    model_key: RawModelKey {
                        friendly_model: pool_lookup(pool, row.1),
                        raw_model_enum: pool_lookup(pool, row.2),
                        response_model: pool_lookup(pool, row.3),
                        raw_api_provider: pool_lookup(pool, row.4),
                    },
                    exact_turn_count: row.5,
                    estimated_turn_count: row.6,
                    exact,
                    estimated,
                });
            }
            loaded_entries.insert(
                conv_id,
                CachedConversationTelemetry {
                    fingerprint: p_entry.fp,
                    data_source: p_entry.src,
                    db_step_count: p_entry.steps,
                    created_at_ms: p_entry.cat,
                    is_native_swarm_worker: p_entry.sw,
                    groups,
                },
            );
        }
        self.telemetry_cache = loaded_entries;

        for (k, v) in parsed.dynamic_enum_map {
            self.dynamic_enum_map.entry(k).or_insert(v);
        }

        for (k, a) in parsed.annotations {
            self.annotation_cache.insert(
                k,
                AnnotationCacheEntry {
                    size: a.s,
                    mtime_ms: a.m,
                    meta: AnnotationMeta {
                        title: a.t,
                        last_view_ms: a.v,
                    },
                },
            );
        }

        for (k, r) in parsed.rpc_sessions {
            self.rpc_meta_cache.insert(
                k,
                RpcMetaCacheEntry {
                    manifest_size: r.ms,
                    manifest_mtime_ms: r.mm,
                    usage_size: r.us,
                    usage_mtime_ms: r.um,
                    meta: RpcSessionMeta {
                        title: r.t,
                        step_count: r.sc,
                        last_modified_ms: r.lm,
                        has_usage: r.hu,
                        usage_size: r.us,
                        usage_mtime_ms: r.um,
                        enums: r.en,
                    },
                },
            );
        }

        for (dir_id, sd) in parsed.swarm_dirs {
            let cid_fallback = sd.cid.clone().unwrap_or_else(|| dir_id.clone());
            let workers =
                sd.w.into_iter()
                    .map(|pw| {
                        (
                            pw.wid,
                            SwarmWorkerMeta {
                                coordinator_id: cid_fallback.clone(),
                                agent_name: pw.ag,
                                model_hint: pw.mh,
                            },
                        )
                    })
                    .collect();
            self.swarm_dir_cache.insert(
                dir_id,
                SwarmDirCacheEntry {
                    size: sd.s,
                    mtime_ms: sd.m,
                    coordinator_id: sd.cid,
                    workers,
                },
            );
        }
    }

    fn persist_disk_cache(&self) {
        if !self.persist_cache {
            return;
        }
        // Strict safety guard: NEVER write anything inside `self.data_dir`.
        if self.cache_dir == self.data_dir || self.cache_dir.starts_with(&self.data_dir) {
            return;
        }
        if std::fs::create_dir_all(&self.cache_dir).is_err() {
            return;
        }

        let mut interner = StringInterner::default();
        let mut entries = HashMap::with_capacity(self.telemetry_cache.len());

        for (conv_id, tel) in &self.telemetry_cache {
            let mut g_tuples = Vec::with_capacity(tel.groups.len());
            for g in &tel.groups {
                let ts = g.max_ts_ms.unwrap_or(NO_TIMESTAMP_SENTINEL);
                let f_id = interner.intern(g.model_key.friendly_model.as_deref());
                let e_id = interner.intern(g.model_key.raw_model_enum.as_deref());
                let r_id = interner.intern(g.model_key.response_model.as_deref());
                let p_id = interner.intern(g.model_key.raw_api_provider.as_deref());
                g_tuples.push(CompactGroupTuple(
                    ts,
                    f_id,
                    e_id,
                    r_id,
                    p_id,
                    g.exact_turn_count,
                    g.estimated_turn_count,
                    g.exact.input_tokens,
                    g.exact.output_tokens,
                    g.exact.thinking_tokens,
                    g.exact.cache_read_tokens,
                    g.exact.cache_write_tokens,
                    g.estimated.input_tokens,
                    g.estimated.output_tokens,
                ));
            }
            entries.insert(
                conv_id.clone(),
                PersistedConvEntry {
                    fp: tel.fingerprint,
                    src: tel.data_source,
                    steps: tel.db_step_count,
                    cat: tel.created_at_ms,
                    sw: tel.is_native_swarm_worker,
                    g: g_tuples,
                },
            );
        }

        let annotations = self
            .annotation_cache
            .iter()
            .map(|(k, a)| {
                (
                    k.clone(),
                    PersistedAnnotationEntry {
                        s: a.size,
                        m: a.mtime_ms,
                        t: a.meta.title.clone(),
                        v: a.meta.last_view_ms,
                    },
                )
            })
            .collect();

        let rpc_sessions = self
            .rpc_meta_cache
            .iter()
            .map(|(k, r)| {
                (
                    k.clone(),
                    PersistedRpcEntry {
                        ms: r.manifest_size,
                        mm: r.manifest_mtime_ms,
                        us: r.usage_size,
                        um: r.usage_mtime_ms,
                        hu: r.meta.has_usage,
                        t: r.meta.title.clone(),
                        sc: r.meta.step_count,
                        lm: r.meta.last_modified_ms,
                        en: r.meta.enums.clone(),
                    },
                )
            })
            .collect();

        let swarm_dirs = self
            .swarm_dir_cache
            .iter()
            .map(|(k, sd)| {
                (
                    k.clone(),
                    PersistedSwarmDirEntry {
                        s: sd.size,
                        m: sd.mtime_ms,
                        cid: sd.coordinator_id.clone(),
                        w: sd
                            .workers
                            .iter()
                            .map(|(wid, wm)| PersistedSwarmWorker {
                                wid: wid.clone(),
                                ag: wm.agent_name.clone(),
                                mh: wm.model_hint.clone(),
                            })
                            .collect(),
                    },
                )
            })
            .collect();

        let payload = PersistedScanCacheFile {
            schema_version: CACHE_SCHEMA_VERSION,
            string_pool: interner.pool,
            entries,
            dynamic_enum_map: self.dynamic_enum_map.clone(),
            annotations,
            rpc_sessions,
            swarm_dirs,
        };

        let Ok(serialized) = serde_json::to_vec(&payload) else {
            return;
        };
        let dest = self.cache_file_path();
        let tmp = self.cache_dir.join(format!(
            ".scan-cache-v3.tmp-{}-{}",
            std::process::id(),
            Utc::now().timestamp_subsec_nanos()
        ));
        if std::fs::write(&tmp, serialized).is_ok() {
            if std::fs::rename(&tmp, &dest).is_err() {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::rename(&tmp, &dest);
            }
            let _ = std::fs::remove_file(&tmp);
            // Clean up obsolete 25MB v2 cache inside Cockpit Tools' cache folder.
            let legacy = self.legacy_v2_cache_file_path();
            if legacy.exists() {
                let _ = std::fs::remove_file(legacy);
            }
        }
    }

    fn scan_annotations(&mut self) -> (HashMap<String, AnnotationMeta>, bool) {
        let ann_dir = self.data_dir.join("annotations");
        let Ok(entries) = std::fs::read_dir(&ann_dir) else {
            return (HashMap::new(), false);
        };

        let mut out = HashMap::new();
        let mut dirty = false;

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name_str) = file_name.to_str() else {
                continue;
            };
            let Some(stem) = name_str.strip_suffix(".pbtxt") else {
                continue;
            };
            if stem.is_empty() || stem.starts_with('.') {
                continue;
            }
            let Some((size, mtime_ms)) = dir_entry_size_mtime(&entry) else {
                continue;
            };

            if let Some(cached) = self.annotation_cache.get(stem) {
                if cached.size == size && cached.mtime_ms == mtime_ms {
                    out.insert(stem.to_string(), cached.meta.clone());
                    continue;
                }
            }

            let path = ann_dir.join(name_str);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let meta = parse_annotation_pbtxt(&content);
                self.annotation_cache.insert(
                    stem.to_string(),
                    AnnotationCacheEntry {
                        size,
                        mtime_ms,
                        meta: meta.clone(),
                    },
                );
                out.insert(stem.to_string(), meta);
                dirty = true;
            }
        }

        let prev_len = self.annotation_cache.len();
        self.annotation_cache.retain(|k, _| out.contains_key(k));
        if self.annotation_cache.len() != prev_len {
            dirty = true;
        }

        (out, dirty)
    }

    fn scan_rpc_sessions(&mut self) -> (HashMap<String, RpcSessionMeta>, PathBuf, usize, bool) {
        let rpc_root = self
            .data_dir
            .join(".token-monitor")
            .join("rpc-cache")
            .join("v1");
        let Ok(entries) = std::fs::read_dir(&rpc_root) else {
            return (HashMap::new(), rpc_root, 0, false);
        };

        let mut sessions = HashMap::new();
        let mut count = 0usize;
        let mut dirty = false;

        struct RpcMissTask {
            id: String,
            dir_path: PathBuf,
            has_manifest: bool,
            m_size: u64,
            m_mtime: u64,
            has_usage: bool,
            u_size: u64,
            u_mtime: u64,
        }
        let mut misses: Vec<RpcMissTask> = Vec::new();

        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }
            let file_name = entry.file_name();
            let Some(id) = file_name.to_str() else {
                continue;
            };
            if id.is_empty() || id.starts_with('.') {
                continue;
            }

            let dir_path = rpc_root.join(id);
            let manifest_stat = safe_stat_wal(&dir_path.join("manifest.json"));
            let usage_stat = safe_stat_wal(&dir_path.join("usage.jsonl"));

            if manifest_stat.is_none() && usage_stat.is_none() {
                continue;
            }
            count += 1;

            let (m_size, m_mtime) = manifest_stat.unwrap_or((0, 0));
            let (u_size, u_mtime) = usage_stat.unwrap_or((0, 0));

            if let Some(cached) = self.rpc_meta_cache.get(id) {
                if cached.manifest_size == m_size
                    && cached.manifest_mtime_ms == m_mtime
                    && cached.usage_size == u_size
                    && cached.usage_mtime_ms == u_mtime
                {
                    for (k, v) in &cached.meta.enums {
                        self.dynamic_enum_map.insert(k.clone(), v.clone());
                    }
                    sessions.insert(id.to_string(), cached.meta.clone());
                    continue;
                }
            }

            misses.push(RpcMissTask {
                id: id.to_string(),
                dir_path,
                has_manifest: manifest_stat.is_some(),
                m_size,
                m_mtime,
                has_usage: usage_stat.is_some(),
                u_size,
                u_mtime,
            });
        }

        if !misses.is_empty() {
            dirty = true;
            let miss_slice = misses.as_slice();
            let resolved_misses: Vec<(RpcMissTask, RpcSessionMeta)> = if miss_slice.len() <= 2 {
                misses
                    .into_iter()
                    .map(|t| {
                        let meta = read_rpc_session_miss(
                            &t.dir_path,
                            t.has_manifest,
                            t.has_usage,
                            t.u_size,
                            t.u_mtime,
                        );
                        (t, meta)
                    })
                    .collect()
            } else {
                let workers = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .clamp(1, 16)
                    .min(miss_slice.len());
                let cursor = AtomicUsize::new(0);
                let parsed_metas: Vec<(usize, RpcSessionMeta)> = std::thread::scope(|scope| {
                    let mut handles = Vec::with_capacity(workers);
                    for _ in 0..workers {
                        let cursor_ref = &cursor;
                        handles.push(scope.spawn(move || {
                            let mut part = Vec::new();
                            loop {
                                let i = cursor_ref.fetch_add(1, AtomicOrdering::Relaxed);
                                if i >= miss_slice.len() {
                                    break;
                                }
                                let t = &miss_slice[i];
                                let meta = read_rpc_session_miss(
                                    &t.dir_path,
                                    t.has_manifest,
                                    t.has_usage,
                                    t.u_size,
                                    t.u_mtime,
                                );
                                part.push((i, meta));
                            }
                            part
                        }));
                    }
                    let mut all = Vec::with_capacity(miss_slice.len());
                    for h in handles {
                        if let Ok(p) = h.join() {
                            all.extend(p);
                        }
                    }
                    all
                });
                let mut by_idx: Vec<Option<RpcSessionMeta>> = vec![None; misses.len()];
                for (i, meta) in parsed_metas {
                    by_idx[i] = Some(meta);
                }
                misses
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, t)| by_idx[i].take().map(|m| (t, m)))
                    .collect()
            };

            for (t, meta) in resolved_misses {
                for (k, v) in &meta.enums {
                    self.dynamic_enum_map.insert(k.clone(), v.clone());
                }
                self.rpc_meta_cache.insert(
                    t.id.clone(),
                    RpcMetaCacheEntry {
                        manifest_size: t.m_size,
                        manifest_mtime_ms: t.m_mtime,
                        usage_size: t.u_size,
                        usage_mtime_ms: t.u_mtime,
                        meta: meta.clone(),
                    },
                );
                sessions.insert(t.id, meta);
            }
        }

        let prev_len = self.rpc_meta_cache.len();
        self.rpc_meta_cache.retain(|k, _| sessions.contains_key(k));
        if self.rpc_meta_cache.len() != prev_len {
            dirty = true;
        }

        (sessions, rpc_root, count, dirty)
    }

    pub fn scan(&mut self, force_full: bool) -> ScanResult {
        let start = Instant::now();
        let scanned_at = format_utc_datetime(Utc::now());
        let data_dir_str = self.data_dir.to_string_lossy().to_string();
        let data_dir_exists = self.data_dir.exists();

        if !data_dir_exists {
            return ScanResult {
                scan: ScanMetadata {
                    data_dir: data_dir_str,
                    data_dir_exists: false,
                    scanned_at,
                    scan_duration_ms: start.elapsed().as_millis() as u64,
                    total_db_files: 0,
                    cache_hits: 0,
                    cache_misses: 0,
                    rpc_cache_sessions_found: 0,
                    brain_dirs_found: 0,
                    warnings: Vec::new(),
                },
                data_dir: self.data_dir.clone(),
                dynamic_enum_map: HashMap::new(),
                conversations: Vec::new(),
                index_by_id: HashMap::new(),
            };
        }

        if force_full {
            self.telemetry_cache.clear();
            self.annotation_cache.clear();
            self.rpc_meta_cache.clear();
            self.swarm_dir_cache.clear();
            self.cache_loaded_from_disk = true;
        } else {
            self.ensure_disk_cache_loaded();
        }

        let mut warnings = Vec::new();
        let mut aux_cache_dirty = false;

        let summaries_db_path = self.data_dir.join("conversation_summaries.db");
        let summary_rows = if summaries_db_path.is_file() {
            match read_conversation_summaries_db(&summaries_db_path) {
                Ok(rows) => rows,
                Err(err) => {
                    warnings.push(format!("conversation_summaries.db: {}", err));
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        let (annotations, ann_dirty) = self.scan_annotations();
        if ann_dirty {
            aux_cache_dirty = true;
        }

        // Single-pass `conversations/` scan collecting both `.db` and `.db-wal` attributes directly
        // from `DirEntry::metadata()` (zero `CreateFileW` / `GetFileAttributesExW` per-file calls!).
        let conv_dir = self.data_dir.join("conversations");
        let mut raw_dbs: HashMap<String, (PathBuf, u64, u64)> = HashMap::new();
        let mut raw_wals: HashMap<String, (u64, u64)> = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&conv_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let Some(name_str) = file_name.to_str() else {
                    continue;
                };
                if name_str.starts_with('.') {
                    continue;
                }
                if let Some(stem) = name_str.strip_suffix(".db") {
                    if !stem.is_empty() {
                        if let Some((sz, mt)) = dir_entry_size_mtime(&entry) {
                            raw_dbs.insert(stem.to_string(), (conv_dir.join(name_str), sz, mt));
                        }
                    }
                } else if let Some(stem) = name_str.strip_suffix(".db-wal") {
                    if !stem.is_empty() {
                        if let Some((wsz, wmt)) = dir_entry_size_mtime(&entry) {
                            raw_wals.insert(stem.to_string(), (wsz, wmt));
                        }
                    }
                }
            }
        }

        let mut db_files: HashMap<String, (PathBuf, u64, u64, u64, u64)> =
            HashMap::with_capacity(raw_dbs.len());
        for (stem, (path, db_size, db_mtime_ms)) in raw_dbs {
            let (wal_size, wal_mtime_ms) = raw_wals.get(&stem).copied().unwrap_or((0, 0));
            db_files.insert(stem, (path, db_size, db_mtime_ms, wal_size, wal_mtime_ms));
        }

        let mut swarm_coordinators: HashSet<String> = HashSet::new();
        let mut swarm_workers: HashMap<String, SwarmWorkerMeta> = HashMap::new();
        let mut transcript_only_files: HashMap<String, (PathBuf, u64, u64)> = HashMap::new();
        let mut seen_swarm_dirs: HashSet<String> = HashSet::new();
        let mut brain_dirs_found = 0usize;

        let brain_root = self.data_dir.join("brain");
        if let Ok(entries) = std::fs::read_dir(&brain_root) {
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    continue;
                }
                let file_name = entry.file_name();
                let Some(id) = file_name.to_str() else {
                    continue;
                };
                if id.is_empty() || id.starts_with('.') {
                    continue;
                }
                brain_dirs_found += 1;

                let dir_path = brain_root.join(id);
                let members_path = dir_path
                    .join(".system_generated")
                    .join("swarm")
                    .join("members.json");

                if let Some((m_size, m_mtime)) = safe_stat_wal(&members_path) {
                    seen_swarm_dirs.insert(id.to_string());
                    let cached_hit = self
                        .swarm_dir_cache
                        .get(id)
                        .filter(|c| c.size == m_size && c.mtime_ms == m_mtime);

                    if let Some(hit) = cached_hit {
                        if let Some(ref cid) = hit.coordinator_id {
                            swarm_coordinators.insert(cid.clone());
                        }
                        for (wid, wm) in &hit.workers {
                            swarm_workers.insert(wid.clone(), wm.clone());
                        }
                    } else {
                        let (cid_opt, workers) = parse_swarm_members_file(id, &members_path);
                        if let Some(ref cid) = cid_opt {
                            swarm_coordinators.insert(cid.clone());
                        }
                        for (wid, wm) in &workers {
                            swarm_workers.insert(wid.clone(), wm.clone());
                        }
                        self.swarm_dir_cache.insert(
                            id.to_string(),
                            SwarmDirCacheEntry {
                                size: m_size,
                                mtime_ms: m_mtime,
                                coordinator_id: cid_opt,
                                workers,
                            },
                        );
                        aux_cache_dirty = true;
                    }
                }

                // Only stat `transcript.jsonl` when `<id>.db` does NOT exist!
                // When `<id>.db` exists, `.db`/`.db-wal` fingerprints track all updates and
                // `transcript.jsonl` is constructed lazily only if a step lacks protobuf usage.
                if !db_files.contains_key(id) {
                    let transcript_path = dir_path
                        .join(".system_generated")
                        .join("logs")
                        .join("transcript.jsonl");
                    if let Some((size, mtime_ms)) = safe_stat_wal(&transcript_path) {
                        if size > 0 {
                            transcript_only_files
                                .insert(id.to_string(), (transcript_path, size, mtime_ms));
                        }
                    }
                }
            }
        }
        let prev_swarm_len = self.swarm_dir_cache.len();
        self.swarm_dir_cache
            .retain(|k, _| seen_swarm_dirs.contains(k));
        if self.swarm_dir_cache.len() != prev_swarm_len {
            aux_cache_dirty = true;
        }

        let (rpc_sessions, rpc_root, rpc_cache_sessions_found, rpc_dirty) =
            self.scan_rpc_sessions();
        if rpc_dirty {
            aux_cache_dirty = true;
        }

        let total_db_files = db_files.len();

        let mut all_ids: HashSet<String> = HashSet::with_capacity(total_db_files + 64);
        all_ids.extend(summary_rows.keys().cloned());
        all_ids.extend(db_files.keys().cloned());
        all_ids.extend(rpc_sessions.keys().cloned());
        all_ids.extend(transcript_only_files.keys().cloned());

        let mut sorted_ids: Vec<String> = all_ids.into_iter().collect();
        sorted_ids.sort_unstable();

        let mut cache_hits = 0usize;
        let mut cache_misses = 0usize;
        let mut extracted_by_id: HashMap<String, CachedConversationTelemetry> =
            HashMap::with_capacity(sorted_ids.len());
        let mut miss_tasks: Vec<ExtractionTask> = Vec::new();

        for id in &sorted_ids {
            let db_entry = db_files.get(id);
            let rpc_entry = rpc_sessions.get(id);
            let tr_entry = transcript_only_files.get(id);
            let has_db = db_entry.is_some();

            let (db_path, db_size, db_mtime_ms, wal_size, wal_mtime_ms) = match db_entry {
                Some((p, ds, dm, ws, wm)) => (Some(p.clone()), *ds, *dm, *ws, *wm),
                None => (None, 0, 0, 0, 0),
            };
            let (rpc_size, rpc_mtime_ms) = match rpc_entry {
                Some(r) => (r.usage_size, r.usage_mtime_ms),
                None => (0, 0),
            };
            let (transcript_size, transcript_mtime_ms) = match tr_entry {
                Some((_, s, m)) => (*s, *m),
                None => (0, 0),
            };

            let fp = FileFingerprint {
                db_size,
                db_mtime_ms,
                wal_size,
                wal_mtime_ms,
                rpc_size,
                rpc_mtime_ms,
                transcript_size,
                transcript_mtime_ms,
            };

            if !force_full {
                if let Some(cached) = self.telemetry_cache.get(id) {
                    if cached.fingerprint == fp {
                        if has_db {
                            cache_hits += 1;
                        }
                        extracted_by_id.insert(id.clone(), cached.clone());
                        continue;
                    }
                }
            }

            if has_db {
                cache_misses += 1;
            }

            let rpc_usage_path = rpc_entry
                .filter(|r| r.has_usage)
                .map(|_| rpc_root.join(id).join("usage.jsonl"));
            let transcript_path = match tr_entry {
                Some((p, _, _)) => Some(p.clone()),
                None if has_db => Some(
                    brain_root
                        .join(id)
                        .join(".system_generated")
                        .join("logs")
                        .join("transcript.jsonl"),
                ),
                None => None,
            };

            miss_tasks.push(ExtractionTask {
                conversation_id: id.clone(),
                fingerprint: fp,
                db_path,
                rpc_usage_path,
                transcript_path,
            });
        }

        let outcomes = parallel_extract(&miss_tasks);

        let mut db_warnings: Vec<(String, String)> = Vec::new();
        for outcome in outcomes {
            if let Some(w) = outcome.warning {
                db_warnings.push((outcome.conversation_id.clone(), w));
            }
            self.telemetry_cache
                .insert(outcome.conversation_id.clone(), outcome.telemetry.clone());
            extracted_by_id.insert(outcome.conversation_id, outcome.telemetry);
        }
        db_warnings.sort_by(|a, b| a.0.cmp(&b.0));
        warnings.extend(db_warnings.into_iter().map(|(_, w)| w));

        let prev_tel_len = self.telemetry_cache.len();
        self.telemetry_cache
            .retain(|k, _| extracted_by_id.contains_key(k));
        let tel_pruned = self.telemetry_cache.len() != prev_tel_len;

        if !miss_tasks.is_empty() || force_full || aux_cache_dirty || tel_pruned {
            self.persist_disk_cache();
        }

        let mut parent_by_id: HashMap<String, String> = HashMap::with_capacity(sorted_ids.len());
        let mut agent_by_id: HashMap<String, String> = HashMap::with_capacity(sorted_ids.len());
        let mut kind_by_id: HashMap<String, ConversationKind> =
            HashMap::with_capacity(sorted_ids.len());
        let mut raw_title_by_id: HashMap<String, String> = HashMap::with_capacity(sorted_ids.len());
        let mut base_ws_by_id: HashMap<String, DecodedWorkspace> =
            HashMap::with_capacity(sorted_ids.len());

        for id in &sorted_ids {
            let sum = summary_rows.get(id);
            let ann = annotations.get(id);
            let rpc = rpc_sessions.get(id);
            let worker = swarm_workers.get(id);
            let tel = extracted_by_id.get(id);

            let parent_id = sum
                .map(|s| s.parent_conversation_id.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| worker.map(|w| w.coordinator_id.clone()))
                .unwrap_or_default();

            let agent_name = sum
                .map(|s| s.agent_name.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| worker.map(|w| w.agent_name.clone()))
                .unwrap_or_default();

            let nesting_depth = sum.map(|s| s.nesting_depth).unwrap_or(0);
            let is_native_swarm = tel.is_some_and(|t| t.is_native_swarm_worker);

            let kind = if worker.is_some() || is_native_swarm {
                ConversationKind::Swarm
            } else if !parent_id.is_empty() || !agent_name.is_empty() || nesting_depth > 0 {
                ConversationKind::Subagent
            } else {
                ConversationKind::Main
            };

            let raw_title = sum
                .map(|s| s.title.trim())
                .filter(|s| !s.is_empty())
                .or_else(|| ann.and_then(|a| a.title.as_deref()))
                .or_else(|| rpc.and_then(|r| r.title.as_deref()))
                .unwrap_or("")
                .to_string();

            let ws = match sum {
                Some(s) => decode_workspace_uris(&s.workspace_uris),
                None => decode_workspace_uris(""),
            };

            if !parent_id.is_empty() && kind == ConversationKind::Swarm {
                swarm_coordinators.insert(parent_id.clone());
            }

            parent_by_id.insert(id.clone(), parent_id);
            agent_by_id.insert(id.clone(), agent_name);
            kind_by_id.insert(id.clone(), kind);
            raw_title_by_id.insert(id.clone(), raw_title);
            base_ws_by_id.insert(id.clone(), ws);
        }

        // Memoize `resolve_model_info` across all conversations (~20 unique RawModelKeys instead of 56,660 calls!).
        let mut model_memo: HashMap<RawModelKey, ResolvedModelInfo> = HashMap::with_capacity(64);
        let scanned_at_ms = parse_timestamp_ms(&scanned_at);
        let mut conversations: Vec<ScannedConversation> = Vec::with_capacity(sorted_ids.len());

        for id in &sorted_ids {
            let sum = summary_rows.get(id);
            let ann = annotations.get(id);
            let rpc = rpc_sessions.get(id);
            let worker = swarm_workers.get(id);
            let Some(tel) = extracted_by_id.get(id) else {
                continue;
            };

            let parent_conversation_id = parent_by_id.get(id).cloned().unwrap_or_default();
            let agent_name = agent_by_id.get(id).cloned().unwrap_or_default();
            let kind = kind_by_id
                .get(id)
                .copied()
                .unwrap_or(ConversationKind::Main);
            let title = raw_title_by_id.get(id).cloned().unwrap_or_default();

            let parent_title = if !parent_conversation_id.is_empty() {
                raw_title_by_id
                    .get(&parent_conversation_id)
                    .cloned()
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let id8: String = id.chars().take(8).collect();
            let display_title = if !title.is_empty() {
                title.clone()
            } else if !agent_name.is_empty() && !parent_title.is_empty() {
                format!("[@{}] {}", agent_name, parent_title)
            } else if !agent_name.is_empty() {
                format!("[@{}] #{}", agent_name, id8)
            } else {
                format!("#{}", id8)
            };

            let mut ws = base_ws_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| decode_workspace_uris(""));
            if sum.is_none()
                && ws.workspace_key == UNASSIGNED_WORKSPACE_KEY
                && !parent_conversation_id.is_empty()
            {
                let mut cursor = parent_conversation_id.as_str();
                for _ in 0..4 {
                    if cursor.is_empty() {
                        break;
                    }
                    if let Some(p_ws) = base_ws_by_id.get(cursor) {
                        if p_ws.workspace_key != UNASSIGNED_WORKSPACE_KEY {
                            ws = p_ws.clone();
                            break;
                        }
                    }
                    cursor = parent_by_id.get(cursor).map(String::as_str).unwrap_or("");
                }
            }

            let conv_model_hint = worker.and_then(|w| w.model_hint.clone());
            let conv_model_hint_str = conv_model_hint.as_deref();

            let mut turn_groups: Vec<ResolvedTurnGroup> = Vec::with_capacity(tel.groups.len());
            let mut conv_exact = ExactTokenTotals::default();
            let mut conv_est_input = 0u64;
            let mut conv_est_output = 0u64;
            let mut conv_est_total = 0u64;
            let mut exact_turn_count = 0usize;
            let mut estimated_turn_count = 0usize;
            let mut latest_turn_ms: Option<i64> = None;
            let mut slices_by_model: HashMap<String, ConversationModelSlice> = HashMap::new();

            for cg in &tel.groups {
                let mut resolved = if let Some(cached_res) = model_memo.get(&cg.model_key) {
                    cached_res.clone()
                } else {
                    let r = resolve_model_info(ModelResolutionInput {
                        friendly_model: cg.model_key.friendly_model.as_deref(),
                        raw_model_enum: cg.model_key.raw_model_enum.as_deref(),
                        response_model: cg.model_key.response_model.as_deref(),
                        api_provider: cg.model_key.raw_api_provider.as_deref(),
                        dynamic_enum_map: Some(&self.dynamic_enum_map),
                    });
                    model_memo.insert(cg.model_key.clone(), r.clone());
                    r
                };

                if resolved.model_id == "unknown" {
                    if let Some(hint) = conv_model_hint_str {
                        resolved = resolve_model_info(ModelResolutionInput {
                            friendly_model: Some(hint),
                            api_provider: cg.model_key.raw_api_provider.as_deref(),
                            dynamic_enum_map: Some(&self.dynamic_enum_map),
                            ..Default::default()
                        });
                    }
                }

                if let Some(ms) = cg.max_ts_ms {
                    latest_turn_ms = Some(latest_turn_ms.map_or(ms, |prev| prev.max(ms)));
                }

                let group_turns = cg.exact_turn_count + cg.estimated_turn_count;
                let slice = slices_by_model
                    .entry(resolved.model_id.clone())
                    .or_insert_with(|| ConversationModelSlice {
                        model_id: resolved.model_id.clone(),
                        model_display_name: resolved.display_name.clone(),
                        api_provider: resolved.api_provider.clone(),
                        turn_count: 0,
                        exact: ExactTokenTotals::default(),
                        estimated_total_tokens: 0,
                    });
                slice.turn_count += group_turns;

                if cg.exact_turn_count > 0 {
                    exact_turn_count += cg.exact_turn_count;
                    conv_exact.add_assign(&cg.exact);
                    slice.exact.add_assign(&cg.exact);
                }
                if cg.estimated_turn_count > 0 {
                    estimated_turn_count += cg.estimated_turn_count;
                    conv_est_input = conv_est_input.saturating_add(cg.estimated.input_tokens);
                    conv_est_output = conv_est_output.saturating_add(cg.estimated.output_tokens);
                    conv_est_total = conv_est_total.saturating_add(cg.estimated.total_tokens);
                    slice.estimated_total_tokens = slice
                        .estimated_total_tokens
                        .saturating_add(cg.estimated.total_tokens);
                }

                turn_groups.push(ResolvedTurnGroup {
                    minute_slot: cg.max_ts_ms.map(|ms| ms.div_euclid(60_000)),
                    max_ts_ms: cg.max_ts_ms,
                    model_id: resolved.model_id,
                    model_display_name: resolved.display_name,
                    api_provider: resolved.api_provider,
                    exact_turn_count: cg.exact_turn_count,
                    estimated_turn_count: cg.estimated_turn_count,
                    exact: cg.exact,
                    estimated: cg.estimated,
                });
            }

            let mut model_breakdown: Vec<ConversationModelSlice> =
                slices_by_model.into_values().collect();
            model_breakdown.sort_by(|a, b| {
                b.exact
                    .total_tokens
                    .cmp(&a.exact.total_tokens)
                    .then_with(|| b.estimated_total_tokens.cmp(&a.estimated_total_tokens))
                    .then_with(|| b.turn_count.cmp(&a.turn_count))
                    .then_with(|| a.model_id.cmp(&b.model_id))
            });

            let (primary_model_id, primary_model_display_name, models, model_display_names) =
                if !model_breakdown.is_empty() {
                    let p_id = model_breakdown[0].model_id.clone();
                    let p_name = model_breakdown[0].model_display_name.clone();
                    let m_ids = model_breakdown.iter().map(|s| s.model_id.clone()).collect();
                    let m_names = model_breakdown
                        .iter()
                        .map(|s| s.model_display_name.clone())
                        .collect();
                    (p_id, p_name, m_ids, m_names)
                } else if let Some(hint) = conv_model_hint_str {
                    let r = resolve_model_info(ModelResolutionInput {
                        friendly_model: Some(hint),
                        dynamic_enum_map: Some(&self.dynamic_enum_map),
                        ..Default::default()
                    });
                    if r.model_id != "unknown" {
                        (
                            r.model_id.clone(),
                            r.display_name.clone(),
                            vec![r.model_id],
                            vec![r.display_name],
                        )
                    } else {
                        (
                            "unknown".to_string(),
                            format_model_display_name("unknown"),
                            Vec::new(),
                            Vec::new(),
                        )
                    }
                } else {
                    (
                        "unknown".to_string(),
                        format_model_display_name("unknown"),
                        Vec::new(),
                        Vec::new(),
                    )
                };

            let summary_last_mod_ms = sum.and_then(|s| s.last_modified_ms);
            let mut last_modified_ms: Option<i64> = match (summary_last_mod_ms, latest_turn_ms) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if last_modified_ms.is_none() {
                last_modified_ms = ann
                    .and_then(|a| a.last_view_ms)
                    .or_else(|| rpc.and_then(|r| r.last_modified_ms))
                    .or(tel.created_at_ms);
            }
            if last_modified_ms.is_none() {
                let max_mtime_ms = tel
                    .fingerprint
                    .wal_mtime_ms
                    .max(tel.fingerprint.db_mtime_ms)
                    .max(tel.fingerprint.rpc_mtime_ms)
                    .max(tel.fingerprint.transcript_mtime_ms);
                if max_mtime_ms > 0 {
                    last_modified_ms = i64::try_from(max_mtime_ms).ok();
                }
            }
            let effective_last_mod_ms = last_modified_ms.or(scanned_at_ms);
            let last_modified_time = effective_last_mod_ms
                .and_then(format_utc_ms)
                .unwrap_or_else(|| scanned_at.clone());

            let total_turn_count = exact_turn_count + estimated_turn_count;
            let step_count = tel
                .db_step_count
                .max(sum.map(|s| s.step_count).unwrap_or(0))
                .max(rpc.map(|r| r.step_count).unwrap_or(0))
                .max(total_turn_count);

            let nesting_depth = sum
                .map(|s| s.nesting_depth)
                .unwrap_or(0)
                .max(usize::from(!parent_conversation_id.is_empty()));

            let summary_item = ConversationSummaryItem {
                conversation_id: id.clone(),
                title,
                display_title,
                preview: sum.map(|s| s.preview.clone()).unwrap_or_default(),
                kind,
                has_swarm_children: swarm_coordinators.contains(id),
                agent_name,
                parent_conversation_id,
                parent_title,
                nesting_depth,
                workspace_uri: ws.workspace_uri,
                workspace_path: ws.workspace_path,
                workspace_name: ws.workspace_name,
                project_id: sum.map(|s| s.project_id.clone()).unwrap_or_default(),
                status: sum.map(|s| s.status.clone()).unwrap_or_default(),
                created_at: tel.created_at_ms.and_then(format_utc_ms),
                last_modified_time,
                step_count,
                turn_count: total_turn_count,
                exact_turn_count,
                estimated_turn_count,
                primary_model_id,
                primary_model_display_name,
                models,
                model_display_names,
                data_source: tel.data_source,
                exact: conv_exact,
                estimated: EstimatedTokenTotals {
                    input_tokens: conv_est_input,
                    output_tokens: conv_est_output,
                    total_tokens: conv_est_total,
                    estimated_turn_count,
                },
            };

            conversations.push(ScannedConversation {
                summary: summary_item,
                last_modified_ms: effective_last_mod_ms,
                created_at_ms: tel.created_at_ms,
                model_hint: conv_model_hint,
                turn_groups,
                model_breakdown,
                children: Vec::new(),
            });
        }

        let mut children_map: HashMap<String, Vec<ChildConversationRef>> = HashMap::new();
        for c in &conversations {
            if !c.summary.parent_conversation_id.is_empty() {
                children_map
                    .entry(c.summary.parent_conversation_id.clone())
                    .or_default()
                    .push(ChildConversationRef {
                        conversation_id: c.summary.conversation_id.clone(),
                        title: c.summary.display_title.clone(),
                        agent_name: c.summary.agent_name.clone(),
                        kind: c.summary.kind,
                        exact_total_tokens: c.summary.exact.total_tokens,
                        estimated_total_tokens: c.summary.estimated.total_tokens,
                    });
            }
        }
        for kids in children_map.values_mut() {
            kids.sort_by(|a, b| {
                b.exact_total_tokens
                    .cmp(&a.exact_total_tokens)
                    .then_with(|| b.estimated_total_tokens.cmp(&a.estimated_total_tokens))
                    .then_with(|| a.conversation_id.cmp(&b.conversation_id))
            });
        }
        for c in &mut conversations {
            if let Some(kids) = children_map.remove(&c.summary.conversation_id) {
                if kids.iter().any(|k| k.kind == ConversationKind::Swarm) {
                    c.summary.has_swarm_children = true;
                }
                c.children = kids;
            }
        }

        conversations.sort_by(|a, b| {
            b.summary
                .last_modified_time
                .cmp(&a.summary.last_modified_time)
                .then_with(|| a.summary.conversation_id.cmp(&b.summary.conversation_id))
        });

        let mut index_by_id = HashMap::with_capacity(conversations.len());
        for (idx, c) in conversations.iter().enumerate() {
            index_by_id.insert(c.summary.conversation_id.clone(), idx);
        }

        // keep `format_utc_secs_nanos` linked for external re-exports / tests
        let _ = format_utc_secs_nanos;

        ScanResult {
            scan: ScanMetadata {
                data_dir: data_dir_str,
                data_dir_exists: true,
                scanned_at,
                scan_duration_ms: start.elapsed().as_millis() as u64,
                total_db_files,
                cache_hits,
                cache_misses,
                rpc_cache_sessions_found,
                brain_dirs_found,
                warnings,
            },
            data_dir: self.data_dir.clone(),
            dynamic_enum_map: self.dynamic_enum_map.clone(),
            conversations,
            index_by_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::antigravity_token_usage::protobuf::{
        encode_bytes_field, encode_varint_field,
    };

    #[test]
    fn scans_sqlite_and_rpc_without_touching_antigravity_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().join("agy-data");
        let cache_dir = tmp.path().join("cockpit-cache");
        let conv_dir = data_dir.join("conversations");
        std::fs::create_dir_all(&conv_dir).expect("conv_dir");

        let db_path = conv_dir.join("conv-1.db");
        {
            let conn = Connection::open(&db_path).expect("open sqlite");
            conn.execute_batch(
                "CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, status INTEGER, metadata BLOB, step_payload BLOB);
                 CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB);",
            )
            .expect("create tables");

            let mut ts = Vec::new();
            ts.extend_from_slice(&encode_varint_field(1, 1_731_628_800));
            let mut usage = Vec::new();
            usage.extend_from_slice(&encode_varint_field(1, 1427));
            usage.extend_from_slice(&encode_varint_field(2, 1000));
            usage.extend_from_slice(&encode_varint_field(3, 300));
            usage.extend_from_slice(&encode_varint_field(5, 4000));
            usage.extend_from_slice(&encode_varint_field(7, b"msg-test-1".len() as u64));
            // replace field 7 properly using encode_bytes_field:
            usage.truncate(0);
            usage.extend_from_slice(&encode_varint_field(1, 1427));
            usage.extend_from_slice(&encode_varint_field(2, 1000));
            usage.extend_from_slice(&encode_varint_field(3, 300));
            usage.extend_from_slice(&encode_varint_field(5, 4000));
            usage.extend_from_slice(&encode_bytes_field(7, b"msg-test-1"));
            usage.extend_from_slice(&encode_varint_field(9, 200));
            usage.extend_from_slice(&encode_varint_field(10, 100));

            let mut meta = Vec::new();
            meta.extend_from_slice(&encode_bytes_field(1, &ts));
            meta.extend_from_slice(&encode_bytes_field(9, &usage));

            conn.execute(
                "INSERT INTO steps (idx, step_type, status, metadata, step_payload) VALUES (0, 15, 1, ?1, ?2)",
                rusqlite::params![meta, b"hello"],
            )
            .expect("insert step");
        }

        let mut scanner = AntigravityScanner::new(Some(data_dir.clone()), cache_dir.clone(), true);
        let res1 = scanner.scan(false);
        assert_eq!(res1.scan.total_db_files, 1);
        assert_eq!(res1.scan.cache_misses, 1);
        assert_eq!(res1.conversations.len(), 1);
        let c = &res1.conversations[0];
        assert_eq!(c.summary.primary_model_id, "gemini-4-carbon-high");
        assert_eq!(c.summary.exact.input_tokens, 1000);
        assert_eq!(c.summary.exact.thinking_tokens, 200);
        assert_eq!(c.summary.exact.output_tokens, 100);
        assert_eq!(c.summary.exact.cache_read_tokens, 4000);
        assert_eq!(c.summary.exact.total_tokens, 5300);

        // Verify on-demand detail extraction recovers full per-turn record including message_id
        let detail = build_conversation_detail(&res1, "conv-1").expect("detail");
        assert_eq!(detail.turns.len(), 1);
        assert_eq!(detail.turns[0].message_id.as_deref(), Some("msg-test-1"));
        assert_eq!(detail.turns[0].exact.total_tokens, 5300);

        // Second scan hits cache and reloads cleanly from disk cache
        let mut scanner2 = AntigravityScanner::new(Some(data_dir.clone()), cache_dir.clone(), true);
        let res2 = scanner2.scan(false);
        assert_eq!(res2.scan.cache_hits, 1);
        assert_eq!(res2.scan.cache_misses, 0);
        assert_eq!(res2.conversations[0].summary.exact.total_tokens, 5300);

        // Verify cache was written in `cache_dir` and NOT inside `data_dir`
        assert!(cache_dir.exists());
        assert!(!data_dir.join("scan-cache-v3.json").exists());
    }

    #[test]
    #[ignore]
    fn bench_real_user_antigravity_dir_readonly() {
        use crate::modules::antigravity_token_usage::aggregator::aggregate_dashboard;
        use crate::modules::antigravity_token_usage::types::DashboardQueryRequest;

        let real_dir = get_default_antigravity_data_dir();
        if !real_dir.exists() {
            return;
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_dir = tmp.path().join("bench-cache");

        // 1. Cold scan (zero cache on disk)
        let mut scanner = AntigravityScanner::new(Some(real_dir.clone()), cache_dir.clone(), true);
        let t0 = Instant::now();
        let cold = scanner.scan(false);
        let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let cache_bytes = std::fs::metadata(scanner.cache_file_path())
            .map(|m| m.len())
            .unwrap_or(0);

        // 2. Warm in-memory incremental scan
        let t1 = Instant::now();
        let warm_mem = scanner.scan(false);
        let warm_mem_ms = t1.elapsed().as_secs_f64() * 1000.0;

        // 3. Cold-process restart scan (fresh scanner loading `scan-cache-v3` from disk)
        let mut restart_scanner = AntigravityScanner::new(Some(real_dir), cache_dir, true);
        let t2 = Instant::now();
        let warm_disk = restart_scanner.scan(false);
        let warm_disk_ms = t2.elapsed().as_secs_f64() * 1000.0;

        // 4. Dashboard aggregation speed across all presets
        let t3 = Instant::now();
        let agg_30d = aggregate_dashboard(&warm_disk, None, Utc::now());
        let agg_all = aggregate_dashboard(
            &warm_disk,
            Some(&DashboardQueryRequest {
                preset: Some("all".to_string()),
                ..Default::default()
            }),
            Utc::now(),
        );
        let agg_ms = t3.elapsed().as_secs_f64() * 1000.0;

        // 5. On-demand single-conversation detail extraction on the largest conversation
        let largest_id = warm_disk
            .conversations
            .iter()
            .max_by_key(|c| c.summary.turn_count)
            .map(|c| c.summary.conversation_id.clone());
        let (detail_turns, detail_ms) = if let Some(ref cid) = largest_id {
            let td = Instant::now();
            let d = build_conversation_detail(&warm_disk, cid).expect("largest detail");
            (d.turns.len(), td.elapsed().as_secs_f64() * 1000.0)
        } else {
            (0, 0.0)
        };

        assert_eq!(cold.conversations.len(), warm_disk.conversations.len());
        assert_eq!(
            agg_all.summary.kpi.exact.total_tokens,
            warm_mem
                .conversations
                .iter()
                .map(|c| c.summary.exact.total_tokens)
                .sum::<u64>()
        );

        println!(
            "\n=== ANTIGRAVITY REAL BENCHMARK ===\n\
             DB files: {} | Conversations: {} | Total turns (all): {} | 30d turns: {}\n\
             Cold full scan + v3 cache save: {:.2} ms\n\
             App-restart warm scan (load v3 + WAL check): {:.2} ms\n\
             In-memory incremental WAL check: {:.2} ms\n\
             Dashboard aggregation (30d + all): {:.3} ms\n\
             Largest conversation detail ({} turns): {:.2} ms\n\
             v3 Cache file size: {:.1} KB ({} bytes)\n",
            cold.scan.total_db_files,
            cold.conversations.len(),
            agg_all.summary.kpi.total_turns,
            agg_30d.summary.kpi.total_turns,
            cold_ms,
            warm_disk_ms,
            warm_mem_ms,
            agg_ms,
            detail_turns,
            detail_ms,
            cache_bytes as f64 / 1024.0,
            cache_bytes,
        );
    }
}
