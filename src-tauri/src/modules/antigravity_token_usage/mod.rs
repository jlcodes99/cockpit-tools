//! Antigravity local token usage module.
//!
//! Scans `~/.gemini/antigravity` (or custom data directory) strictly read-only,
//! caching file fingerprints inside Cockpit Tools' own data folder (`antigravity_token_usage/`).
//!
//! Concurrency design:
//! - `SNAPSHOT_STORE` (`RwLock<Option<SnapshotEntry>>`): Lock-free/shared read path for
//!   `query_token_usage` and `get_conversation_detail` — NEVER blocked by an active background scan
//!   once a snapshot exists!
//! - `SCAN_COORDINATOR` (`Mutex<Option<ScannerState>>`): Single-flight scan serializer that
//!   coalesces concurrent or back-to-back incremental `sync_token_usage(false)` requests within
//!   `RECENT_SCAN_REUSE_WINDOW`.

pub mod aggregator;
pub mod models;
pub mod protobuf;
pub mod scanner;
pub mod types;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::modules::account;

use self::aggregator::{aggregate_dashboard, query_conversation_list};
use self::scanner::{
    build_conversation_detail, get_default_antigravity_data_dir, AntigravityScanner,
};
use self::types::{
    AntigravityTokenUsageSettings, ConversationDetailResponse, DashboardBundleResponse,
    DashboardQueryRequest, ScanResult,
};

const STORAGE_SUBDIR: &str = "antigravity_token_usage";
const SETTINGS_FILE_NAME: &str = "settings.json";
const RECENT_SCAN_REUSE_WINDOW: Duration = Duration::from_millis(2_500);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTokenUsageSettings {
    #[serde(default)]
    custom_data_dir: Option<String>,
}

#[derive(Clone)]
struct SnapshotEntry {
    storage_dir: PathBuf,
    custom_data_dir: Option<String>,
    completed_at: Instant,
    scan: Arc<ScanResult>,
}

struct ScannerState {
    storage_dir: PathBuf,
    custom_data_dir: Option<String>,
    scanner: AntigravityScanner,
}

static SNAPSHOT_STORE: OnceLock<RwLock<Option<SnapshotEntry>>> = OnceLock::new();
static SCAN_COORDINATOR: OnceLock<Mutex<Option<ScannerState>>> = OnceLock::new();

fn snapshot_lock() -> &'static RwLock<Option<SnapshotEntry>> {
    SNAPSHOT_STORE.get_or_init(|| RwLock::new(None))
}

fn coordinator_lock() -> &'static Mutex<Option<ScannerState>> {
    SCAN_COORDINATOR.get_or_init(|| Mutex::new(None))
}

fn sanitize_custom_dir(raw: Option<&str>) -> Option<String> {
    raw.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_storage_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(STORAGE_SUBDIR))
}

fn read_persisted_settings(storage_dir: &Path) -> PersistedTokenUsageSettings {
    let path = storage_dir.join(SETTINGS_FILE_NAME);
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<PersistedTokenUsageSettings>(&raw).ok())
        .unwrap_or_default()
}

fn write_persisted_settings(
    storage_dir: &Path,
    settings: &PersistedTokenUsageSettings,
) -> Result<(), String> {
    fs::create_dir_all(storage_dir).map_err(|e| format!("Failed to create settings dir: {e}"))?;
    let mut raw = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    raw.push('\n');

    let dest = storage_dir.join(SETTINGS_FILE_NAME);
    let tmp = storage_dir.join(format!(
        ".{SETTINGS_FILE_NAME}.tmp-{}-{}",
        std::process::id(),
        Utc::now().timestamp_subsec_nanos()
    ));
    let res = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(raw.as_bytes())?;
        f.sync_all()?;
        drop(f);
        if fs::rename(&tmp, &dest).is_err() {
            let _ = fs::remove_file(&dest);
            fs::rename(&tmp, &dest)?;
        }
        Ok(())
    })();
    if let Err(err) = res {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to persist settings: {err}"));
    }
    Ok(())
}

fn build_settings_snapshot(custom_data_dir: Option<&str>) -> AntigravityTokenUsageSettings {
    let clean_custom = sanitize_custom_dir(custom_data_dir);
    let default_dir = get_default_antigravity_data_dir()
        .to_string_lossy()
        .to_string();
    let (effective_data_dir, is_custom_data_dir) = match &clean_custom {
        Some(custom) => (custom.clone(), true),
        None => (default_dir.clone(), false),
    };
    AntigravityTokenUsageSettings {
        custom_data_dir: clean_custom,
        effective_data_dir,
        default_data_dir: default_dir,
        is_custom_data_dir,
    }
}

fn build_bundle(
    scan: &ScanResult,
    custom_data_dir: Option<&str>,
    query: Option<&DashboardQueryRequest>,
) -> DashboardBundleResponse {
    let agg = aggregate_dashboard(scan, query, Utc::now());
    let conversations = query_conversation_list(&agg.filtered_conversations, query);
    let settings = build_settings_snapshot(custom_data_dir);
    DashboardBundleResponse {
        summary: agg.summary,
        conversations,
        settings,
    }
}

fn read_snapshot_if_matching(storage_dir: &Path) -> Option<SnapshotEntry> {
    let guard = snapshot_lock().read().ok()?;
    guard
        .as_ref()
        .filter(|entry| entry.storage_dir == storage_dir)
        .cloned()
}

fn run_coordinated_scan(
    storage_dir: PathBuf,
    rebuild: bool,
    reuse_if_any: bool,
) -> Result<SnapshotEntry, String> {
    let wait_started = Instant::now();

    if !rebuild {
        if let Some(existing) = read_snapshot_if_matching(&storage_dir) {
            if reuse_if_any || existing.completed_at.elapsed() <= RECENT_SCAN_REUSE_WINDOW {
                return Ok(existing);
            }
        }
    }

    let mut coord_guard = coordinator_lock()
        .lock()
        .map_err(|_| "Antigravity token usage scan lock poisoned".to_string())?;

    // Single-flight check after acquiring `SCAN_COORDINATOR`: if another caller just finished
    // scanning while we waited, reuse that fresh snapshot instead of scanning a second time!
    if !rebuild {
        if let Some(existing) = read_snapshot_if_matching(&storage_dir) {
            if reuse_if_any
                || existing.completed_at >= wait_started
                || existing.completed_at.elapsed() <= RECENT_SCAN_REUSE_WINDOW
            {
                return Ok(existing);
            }
        }
    }

    let state = coord_guard.get_or_insert_with(|| {
        let persisted = read_persisted_settings(&storage_dir);
        let custom = sanitize_custom_dir(persisted.custom_data_dir.as_deref());
        ScannerState {
            storage_dir: storage_dir.clone(),
            custom_data_dir: custom.clone(),
            scanner: AntigravityScanner::new(custom.map(PathBuf::from), storage_dir.clone(), true),
        }
    });

    if state.storage_dir != storage_dir {
        let persisted = read_persisted_settings(&storage_dir);
        let custom = sanitize_custom_dir(persisted.custom_data_dir.as_deref());
        state.storage_dir = storage_dir.clone();
        state.custom_data_dir = custom.clone();
        state.scanner =
            AntigravityScanner::new(custom.map(PathBuf::from), storage_dir.clone(), true);
    }

    let scan = Arc::new(state.scanner.scan(rebuild));
    let entry = SnapshotEntry {
        storage_dir,
        custom_data_dir: state.custom_data_dir.clone(),
        completed_at: Instant::now(),
        scan,
    };

    if let Ok(mut snap_guard) = snapshot_lock().write() {
        *snap_guard = Some(entry.clone());
    }

    Ok(entry)
}

/// Fast lock-free query: uses in-memory `Arc<ScanResult>` immediately when available
/// (never waiting on a concurrent background `sync`), or runs/joins a single initial scan.
pub fn query_token_usage(
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    let storage_dir = resolve_storage_dir()?;
    let entry = match read_snapshot_if_matching(&storage_dir) {
        Some(hit) => hit,
        None => run_coordinated_scan(storage_dir, false, true)?,
    };
    Ok(build_bundle(
        &entry.scan,
        entry.custom_data_dir.as_deref(),
        query.as_ref(),
    ))
}

/// Incremental WAL-aware sync (`rebuild == false`, single-flight coalesced) or full rescan (`rebuild == true`).
pub fn sync_token_usage(
    rebuild: bool,
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    let storage_dir = resolve_storage_dir()?;
    let entry = run_coordinated_scan(storage_dir, rebuild, false)?;
    Ok(build_bundle(
        &entry.scan,
        entry.custom_data_dir.as_deref(),
        query.as_ref(),
    ))
}

/// Extracts full per-turn breakdown for a single conversation on demand (~1–2 ms) without blocking scans.
pub fn get_conversation_detail(
    conversation_id: &str,
) -> Result<ConversationDetailResponse, String> {
    let id = conversation_id.trim();
    if id.is_empty() {
        return Err("conversation_id is required".to_string());
    }
    let storage_dir = resolve_storage_dir()?;
    let entry = match read_snapshot_if_matching(&storage_dir) {
        Some(hit) => hit,
        None => run_coordinated_scan(storage_dir, false, true)?,
    };
    let conversation = build_conversation_detail(&entry.scan, id)?;
    Ok(ConversationDetailResponse {
        scan: entry.scan.scan.clone(),
        conversation,
    })
}

pub fn get_token_usage_settings() -> Result<AntigravityTokenUsageSettings, String> {
    let storage_dir = resolve_storage_dir()?;
    if let Some(entry) = read_snapshot_if_matching(&storage_dir) {
        return Ok(build_settings_snapshot(entry.custom_data_dir.as_deref()));
    }
    let persisted = read_persisted_settings(&storage_dir);
    Ok(build_settings_snapshot(
        persisted.custom_data_dir.as_deref(),
    ))
}

pub fn set_custom_data_dir(
    custom_dir: Option<String>,
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    let storage_dir = resolve_storage_dir()?;
    let next_custom = sanitize_custom_dir(custom_dir.as_deref());
    write_persisted_settings(
        &storage_dir,
        &PersistedTokenUsageSettings {
            custom_data_dir: next_custom.clone(),
        },
    )?;

    let mut coord_guard = coordinator_lock()
        .lock()
        .map_err(|_| "Antigravity token usage scan lock poisoned".to_string())?;

    let mut scanner = AntigravityScanner::new(
        next_custom.clone().map(PathBuf::from),
        storage_dir.clone(),
        true,
    );
    let scan = Arc::new(scanner.scan(false));
    *coord_guard = Some(ScannerState {
        storage_dir: storage_dir.clone(),
        custom_data_dir: next_custom.clone(),
        scanner,
    });

    let entry = SnapshotEntry {
        storage_dir,
        custom_data_dir: next_custom,
        completed_at: Instant::now(),
        scan,
    };
    if let Ok(mut snap_guard) = snapshot_lock().write() {
        *snap_guard = Some(entry.clone());
    }
    drop(coord_guard);

    Ok(build_bundle(
        &entry.scan,
        entry.custom_data_dir.as_deref(),
        query.as_ref(),
    ))
}
