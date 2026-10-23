//! Automatic quota synchronization. No API gateway, subscription or WebSocket is
//! needed: runtime activity schedules authoritative usage GETs through the common
//! account refresh queue. Only committed quota snapshots are sent to webviews.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures_util::FutureExt;
use serde::Serialize;
use tauri::Emitter;

use crate::models::codex::{CodexAccount, CodexQuota};
use crate::modules::{codex_account, config, logger};

const EVENT: &str = "codex:quota-updated";
const MIN_INTERVAL: Duration = Duration::from_secs(5);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const ACTIVITY_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_WINDOW: Duration = Duration::from_secs(120);
const MAX_PENDING: usize = 4;
const MAX_PROBES_PER_TICK: usize = 8;
static STARTED: AtomicBool = AtomicBool::new(false);
static DISCOVERY_RUNNING: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
// Store timing only, never credentials, transcript text or full quota responses.
static OBSERVED: LazyLock<Mutex<HashMap<String, (Instant, Duration)>>> =
    LazyLock::new(Default::default);
// Per-email overrides live in the existing frontend settings store. Wait for
// its initial handshake rather than briefly overriding a disabled account.
static DISABLED_EMAILS: LazyLock<Mutex<Option<HashSet<String>>>> = LazyLock::new(Default::default);

#[tauri::command]
pub fn set_codex_quota_sync_disabled_emails(emails: Vec<String>) -> Result<(), String> {
    let mut policy = DISABLED_EMAILS
        .lock()
        .map_err(|_| "CODEX_QUOTA_SYNC_POLICY_LOCK".to_string())?;
    *policy = Some(emails.into_iter().collect());
    Ok(())
}

#[derive(Clone, Serialize)]
pub(crate) struct QuotaUpdated {
    account_id: String,
    remote_account_id: Option<String>,
    organization_id: Option<String>,
    token_generation: u64,
    quota: CodexQuota,
    plan_type: Option<String>,
    usage_updated_at: i64,
    sequence: u64,
}

pub(crate) fn publish(account: &CodexAccount) {
    let (Some(quota), Some(updated_at)) = (&account.quota, account.usage_updated_at) else {
        return;
    };
    if let Ok(mut observed) = OBSERVED.lock() {
        observed.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
        observed.insert(
            account.id.clone(),
            (Instant::now(), dynamic_interval(Some(quota))),
        );
    }
    let Some(app) = crate::get_app_handle() else {
        return;
    };
    let payload = QuotaUpdated {
        account_id: account.id.clone(),
        remote_account_id: account.account_id.clone(),
        organization_id: account.organization_id.clone(),
        token_generation: account.token_generation,
        quota: quota.clone(),
        plan_type: account.plan_type.clone(),
        usage_updated_at: updated_at,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
    };
    if let Err(error) = app.emit(EVENT, payload) {
        logger::log_warn(&format!("[Codex Quota Sync] quota event failed: {error}"));
    }
}

pub(crate) fn observed_since(account_id: &str, since: Instant) -> bool {
    OBSERVED
        .lock()
        .ok()
        .is_some_and(|observed| observed.get(account_id).is_some_and(|(at, _)| *at >= since))
}

fn dynamic_interval(quota: Option<&CodexQuota>) -> Duration {
    let Some(quota) = quota else {
        return MIN_INTERVAL;
    };
    let remaining = [
        (quota.hourly_window_present, quota.hourly_percentage),
        (quota.weekly_window_present, quota.weekly_percentage),
    ]
    .into_iter()
    .filter(|(present, _)| *present != Some(false))
    .map(|(_, value)| value.clamp(0, 100))
    .min()
    .unwrap_or(100);
    Duration::from_secs(((remaining as u64 * 30) / 100).clamp(5, 30))
}

fn failure_backoff(failures: u32, error: &str) -> Duration {
    let hinted = error
        .split("[retry_after_seconds:")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    Duration::from_secs((5_u64 << failures.min(6)).min(300).max(hinted.min(3600)))
}

fn plan_key(plan: Option<&str>) -> &'static str {
    let lower = plan.unwrap_or("").to_ascii_lowercase();
    let words: HashSet<_> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    for key in ["enterprise", "business", "team"] {
        if words.contains(key) {
            return key;
        }
    }
    if ["edu", "education", "k12"]
        .iter()
        .any(|key| words.contains(key))
    {
        return "edu_k12";
    }
    for key in ["go", "plus"] {
        if words.contains(key) {
            return key;
        }
    }
    if ["pro", "prolite", "promax"]
        .iter()
        .any(|key| words.contains(key))
    {
        return "pro";
    }
    if words.contains("free") {
        "free"
    } else {
        "unknown"
    }
}

fn policy_allows(id: &str, policy: &(bool, HashSet<String>, HashSet<String>)) -> bool {
    !policy.2.contains(id) && (policy.0 || policy.1.contains(id))
}

fn eligible(
    account: &CodexAccount,
    settings: &config::UserConfig,
    policy: &(bool, HashSet<String>, HashSet<String>),
) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
        && !codex_account::is_pending_oauth_account(account)
        && policy_allows(&account.id, policy)
        && DISABLED_EMAILS.lock().ok().is_some_and(|overrides| {
            overrides.as_ref().is_some_and(|emails| {
                policy.1.contains(&account.id) || !emails.contains(&account.email)
            })
        })
        && settings
            .codex_auto_refresh_plan_types
            .iter()
            .any(|key| key == plan_key(account.plan_type.as_deref()))
}

pub(crate) fn is_eligible(id: &str) -> bool {
    let settings = config::get_user_config();
    let policy =
        codex_account::quota_sync_refresh_allowed_ids(settings.codex_auto_refresh_minutes > 0);
    codex_account::load_account(id).is_some_and(|account| eligible(&account, &settings, &policy))
}

#[derive(Default)]
struct RefreshState {
    next_due: Option<Instant>,
    last_started: Option<Instant>,
    last_observed: Option<Instant>,
    failures: u32,
    activity_pending: bool,
    in_flight: bool,
    last_activity: Option<Instant>,
}

impl RefreshState {
    fn observe(&mut self, at: Instant, interval: Duration) {
        if self.last_observed.is_none_or(|previous| at > previous) {
            self.last_observed = Some(at);
            self.failures = 0;
            self.next_due = Some(at + interval);
        }
    }
    fn due(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        if self
            .last_activity
            .is_some_and(|at| now.saturating_duration_since(at) > ACTIVE_WINDOW)
        {
            return false;
        }
        if self
            .last_started
            .is_some_and(|at| now.saturating_duration_since(at) < MIN_INTERVAL)
        {
            return false;
        }
        if self
            .last_observed
            .is_some_and(|at| now.saturating_duration_since(at) < MIN_INTERVAL)
        {
            return false;
        }
        // Activity cannot bypass Retry-After or exponential backoff.
        (self.activity_pending && self.failures == 0) || self.next_due.is_none_or(|at| now >= at)
    }
    fn finish(&mut self, now: Instant, result: &Result<CodexQuota, String>) {
        self.in_flight = false;
        match result {
            Ok(quota) => {
                self.failures = 0;
                self.next_due = Some(now + dynamic_interval(Some(quota)));
            }
            Err(error) => {
                self.failures = self.failures.saturating_add(1);
                self.next_due = Some(now + failure_backoff(self.failures, error));
            }
        }
    }
}

// Inspect only recent session-file metadata. No transcript content is read and
// historical cached token_count/rate_limits records cannot become quota evidence.
// Bounded scanning runs on the blocking pool, outside startup/UI and state locks.
#[derive(Default)]
struct ActivityProbe {
    previous: HashMap<PathBuf, (u64, Option<SystemTime>)>,
    initialized: bool,
}

/// The official indexes make this a bounded read even for a large history.
/// This also finds resumed rollouts still stored under an old creation date.
/// Missing/busy/unsupported databases fall back to recent directory metadata.
fn recent_rollout_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for db in [
        home.join("sqlite/state_5.sqlite"),
        home.join("state_5.sqlite"),
    ] {
        if !db.is_file() {
            continue;
        }
        let Ok(connection) =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        let _ = connection.busy_timeout(Duration::from_millis(20));
        for (index, column) in [
            ("idx_threads_updated_at_ms", "updated_at_ms"),
            ("idx_threads_updated_at", "updated_at"),
        ] {
            let query = format!("SELECT rollout_path FROM threads INDEXED BY {index} ORDER BY {column} DESC LIMIT 16");
            let Ok(mut stmt) = connection.prepare(&query) else {
                continue;
            };
            let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
                continue;
            };
            for path in rows.flatten() {
                let path = PathBuf::from(path);
                let path = if path.is_absolute() {
                    path
                } else {
                    home.join(path)
                };
                // Only inspect session metadata in this profile, never follow
                // a malformed database reference to arbitrary user files.
                if path.starts_with(home.join("sessions"))
                    && !path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                {
                    paths.push(path);
                }
            }
            break;
        }
    }
    paths
}

impl ActivityProbe {
    fn scan(&mut self, home: &Path, now: chrono::DateTime<chrono::Utc>) -> bool {
        let mut current = HashMap::new();
        let local_day = now.with_timezone(&chrono::Local).date_naive();
        let dates = HashSet::from([
            now.date_naive(),
            (now - chrono::Duration::days(1)).date_naive(),
            local_day,
            local_day - chrono::Duration::days(1),
        ]);
        let mut candidates = recent_rollout_paths(home);
        candidates.extend(
            self.previous
                .iter()
                .filter(|(_, (_, modified))| {
                    modified.is_some_and(|at| {
                        at.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            >= now
                                .timestamp()
                                .saturating_sub(ACTIVE_WINDOW.as_secs() as i64)
                                .max(0) as u64
                    })
                })
                .map(|(path, _)| path.clone()),
        );
        for day in dates {
            let dir = home
                .join("sessions")
                .join(day.format("%Y/%m/%d").to_string());
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            // A pathological session directory cannot monopolize the worker.
            candidates.extend(entries.take(256).flatten().map(|entry| entry.path()));
        }
        for path in candidates {
            if path.extension().is_none_or(|ext| ext != "jsonl") {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            current.insert(path, (meta.len(), meta.modified().ok()));
        }
        let changed = self.initialized
            && current.iter().any(|(path, stamp)| {
                self.previous.get(path) != Some(stamp)
                    && stamp.1.is_some_and(|at| {
                        at.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            >= now.timestamp().saturating_sub(30).max(0) as u64
                    })
            });
        self.previous = current;
        self.initialized = true;
        changed
    }
}

#[derive(Default)]
struct Discovery {
    profiles: Vec<(PathBuf, String)>,
    probes: HashMap<(PathBuf, String), ActivityProbe>,
    running: HashSet<String>,
    runtime: Option<std::sync::Arc<codex_account::CodexQuotaRuntimeSnapshot>>,
    probe_cursor: usize,
}

struct DiscoveryFlight;
impl Drop for DiscoveryFlight {
    fn drop(&mut self) {
        DISCOVERY_RUNNING.store(false, Ordering::Release);
    }
}

async fn discover() -> Result<Discovery, String> {
    if DISCOVERY_RUNNING.swap(true, Ordering::AcqRel) {
        return Err("CODEX_QUOTA_DISCOVERY_BUSY".into());
    }
    // Keep the flight guard inside blocking work: a timed-out OS probe must
    // finish before another probe can start, even after its awaiter is dropped.
    let task = tokio::task::spawn_blocking(move || {
        let _flight = DiscoveryFlight;
        let runtime = codex_account::CodexQuotaRuntimeSnapshot::capture_blocking();
        let settings = config::get_user_config();
        let policy =
            codex_account::quota_sync_refresh_allowed_ids(settings.codex_auto_refresh_minutes > 0);
        let accounts = codex_account::list_accounts();
        let mut profiles = runtime.quota_activity_profiles(&accounts);
        profiles.sort();
        let running = runtime
            .running_oauth_account_ids()
            .map(|ids| ids.clone())
            .unwrap_or_default();
        let allowed: HashMap<_, _> = accounts
            .iter()
            .filter(|a| eligible(a, &settings, &policy))
            .map(|a| (a.id.clone(), dynamic_interval(a.quota.as_ref())))
            .collect();
        Ok(Discovery {
            profiles: profiles
                .into_iter()
                .filter(|(_, id)| allowed.contains_key(id))
                .collect(),
            probes: HashMap::new(),
            running,
            runtime: Some(std::sync::Arc::new(runtime)),
            probe_cursor: 0,
        })
    });
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .map_err(|_| "CODEX_QUOTA_DISCOVERY_TIMEOUT".to_string())?
        .map_err(|e| e.to_string())?
}

pub(crate) fn ensure_started() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async {
        let mut discovery = Discovery::default();
        let mut states: HashMap<String, RefreshState> = HashMap::new();
        let mut discovery_due = Instant::now();
        let mut jobs = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(ACTIVITY_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                Some(completed) = jobs.join_next(), if !jobs.is_empty() => {
                    if let Ok((id, result)) = completed {
                        if let Some(state) = states.get_mut(&id) { state.finish(Instant::now(), &result); }
                        if let Err(error) = result {
                            // Error details stay in the existing quota refresh diagnostics.
                            logger::log_warn(&format!("[Codex Quota Sync] refresh deferred/failed: account_id={id}, retry_in={}s",
                                failure_backoff(states.get(&id).map_or(1, |s| s.failures), &error).as_secs()));
                        }
                    }
                }
                _ = tick.tick() => {
                    if Instant::now() >= discovery_due {
                        match discover().await {
                            Ok(mut next) => {
                                next.probe_cursor = discovery.probe_cursor;
                                for profile in &next.profiles {
                                    if let Some(probe) = discovery.probes.remove(profile) { next.probes.insert(profile.clone(), probe); }
                                }
                                discovery = next;
                            }
                            Err(error) => {
                                logger::log_warn(&format!("[Codex Quota Sync] runtime discovery failed: {error}"));
                                // Retain quota cache, but never schedule on an unverified runtime.
                                discovery = Discovery::default();
                            }
                        }
                        discovery_due = Instant::now() + DISCOVERY_INTERVAL;
                    }
                    let active: HashSet<_> = discovery.profiles.iter().map(|(_, id)| id.clone()).collect();
                    states.retain(|id, state| active.contains(id) || state.in_flight);
                    // Move probes into one bounded blocking job, never hold a global
                    // lock while touching files. A switch starts a fresh baseline.
                    let mut moved = std::mem::take(&mut discovery);
                    discovery = match tokio::task::spawn_blocking(move || {
                        let mut changed = HashSet::new();
                        let total = moved.profiles.len();
                        for offset in 0..total.min(MAX_PROBES_PER_TICK) {
                            let (home, id) = &moved.profiles[(moved.probe_cursor + offset) % total];
                            if moved.probes.entry((home.clone(), id.clone())).or_default().scan(home, chrono::Utc::now()) {
                                changed.insert(id.clone());
                            }
                        }
                        moved.probe_cursor = if total == 0 { 0 } else { (moved.probe_cursor + MAX_PROBES_PER_TICK) % total };
                        (moved, changed)
                    }).await {
                        Ok((next, changed)) => {
                            for id in changed {
                                let state = states.entry(id).or_default();
                                state.activity_pending = true;
                                state.last_activity = Some(Instant::now());
                            }
                            next
                        }
                        Err(_) => Discovery::default(),
                    };
                    let now = Instant::now();
                    let observed = OBSERVED.lock().map(|map| map.clone()).unwrap_or_default();
                    for id in active {
                        let state = states.entry(id.clone()).or_insert_with(|| RefreshState {
                            last_activity: Some(if discovery.running.contains(&id) { now } else {
                                now.checked_sub(ACTIVE_WINDOW + Duration::from_secs(1)).unwrap_or(now)
                            }),
                            ..Default::default()
                        });
                        if let Some((at, interval)) = observed.get(&id) { state.observe(*at, *interval); }
                        if jobs.len() >= MAX_PENDING || !state.due(now) { continue; }
                        state.in_flight = true;
                        state.last_started = Some(now);
                        state.activity_pending = false;
                        let runtime = discovery.runtime.clone();
                        jobs.spawn(async move {
                            let result = std::panic::AssertUnwindSafe(
                                crate::modules::codex_quota_refresh_scheduler::refresh_active_account(&id, runtime)
                            ).catch_unwind().await.unwrap_or_else(|_| Err("CODEX_QUOTA_SYNC_INTERRUPTED".into()));
                            (id, result)
                        });
                    }
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "codex_quota_sync_tests.rs"]
mod tests;
