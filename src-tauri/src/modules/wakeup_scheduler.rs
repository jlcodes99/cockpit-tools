use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::time::sleep;

use crate::modules;

const DEFAULT_PROMPT: &str = "hi";
const RESET_TRIGGER_COOLDOWN_MS: i64 = 10 * 60 * 1000;
const RESET_SAFETY_MARGIN_MS: i64 = 2 * 60 * 1000;
const WAKEUP_TASKS_FILE: &str = "wakeup_tasks.json";

#[path = "antigravity_wakeup_store.rs"]
mod store;
#[path = "antigravity_wakeup_quota_refresh.rs"]
mod quota_refresh;
static RULES_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupTaskInput {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(
        default,
        alias = "execution_mode",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_mode: Option<String>,
    #[serde(
        default,
        alias = "confirm_timeout_minutes",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirm_timeout_minutes: Option<i32>,
    pub schedule: ScheduleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedWakeupState {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    tasks: Vec<WakeupTaskInput>,
    #[serde(default)]
    webview_seeded: bool,
    #[serde(default)]
    rules_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleConfig {
    pub repeat_mode: String,
    pub daily_times: Option<Vec<String>>,
    pub weekly_days: Option<Vec<i32>>,
    pub weekly_times: Option<Vec<String>>,
    pub interval_hours: Option<i32>,
    pub interval_start_time: Option<String>,
    pub interval_end_time: Option<String>,
    pub selected_models: Vec<String>,
    pub selected_accounts: Vec<String>,
    pub crontab: Option<String>,
    pub wake_on_reset: Option<bool>,
    pub custom_prompt: Option<String>,
    pub max_output_tokens: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_mode: Option<modules::wakeup::WakeupTransportMode>,
    pub time_window_enabled: Option<bool>,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    pub fallback_times: Option<Vec<String>>,
    pub startup_delay_minutes: Option<i32>,
    pub execution_mode: Option<String>,
    pub confirm_timeout_minutes: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct WakeupTask {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub schedule: ScheduleConfigNormalized,
    pub execution_mode: String,
    pub confirm_timeout_minutes: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleConfigNormalized {
    pub repeat_mode: String,
    pub daily_times: Vec<String>,
    pub weekly_days: Vec<i32>,
    pub weekly_times: Vec<String>,
    pub interval_hours: i32,
    pub interval_start_time: String,
    pub interval_end_time: String,
    pub selected_models: Vec<String>,
    pub selected_accounts: Vec<String>,
    pub crontab: Option<String>,
    pub wake_on_reset: bool,
    pub custom_prompt: Option<String>,
    pub max_output_tokens: i32,
    pub transport_mode: Option<modules::wakeup::WakeupTransportMode>,
    pub time_window_enabled: bool,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    pub fallback_times: Vec<String>,
    pub startup_delay_minutes: Option<i32>,
    pub execution_mode: String,
    pub confirm_timeout_minutes: i32,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct ResetState {
    last_reset_trigger_timestamps: HashMap<String, String>,
    last_reset_trigger_at: HashMap<String, i64>,
    last_reset_remaining: HashMap<String, i32>,
}

#[derive(Default, Clone)]
struct SchedulerState {
    enabled: bool,
    tasks: Vec<WakeupTask>,
    running_tasks: HashSet<String>,
    reset_states: HashMap<String, ResetState>,
    /// 记录每个任务的实际执行时间，不会被前端 sync_state 覆盖
    last_executed_at: HashMap<String, i64>,
    last_claim_at: HashMap<String, i64>,
}

static STATE: OnceLock<Mutex<SchedulerState>> = OnceLock::new();
static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
static STARTUP_TRIGGERED: OnceLock<Mutex<bool>> = OnceLock::new();

#[derive(Debug, Clone)]
struct PendingConfirmation {
    task: WakeupTask,
    trigger_source: String,
    scheduled_at: i64,
    timeout_at: i64,
    due_at: Option<i64>,
}

static PENDING_CONFIRMATIONS: OnceLock<Mutex<HashMap<String, PendingConfirmation>>> =
    OnceLock::new();

fn pending_confirmations() -> &'static Mutex<HashMap<String, PendingConfirmation>> {
    PENDING_CONFIRMATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn state() -> &'static Mutex<SchedulerState> {
    STATE.get_or_init(|| Mutex::new(SchedulerState::default()))
}

fn started_flag() -> &'static Mutex<bool> {
    STARTED.get_or_init(|| Mutex::new(false))
}

fn startup_triggered_flag() -> &'static Mutex<bool> {
    STARTUP_TRIGGERED.get_or_init(|| Mutex::new(false))
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, label: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(err) => {
            modules::logger::log_warn(&format!(
                "[WakeupTasks] 检测到锁中毒，继续使用恢复数据: {}",
                label
            ));
            err.into_inner()
        }
    }
}

fn wakeup_store() -> Result<store::WakeupStore, String> {
    Ok(store::WakeupStore::new(
        modules::account::get_data_dir()?.join(WAKEUP_TASKS_FILE),
    ))
}

fn apply_state(config: &PersistedWakeupState, runtime: HashMap<String, store::Runtime>) {
    let mut guard = lock_or_recover(state(), "wakeup state lock");
    guard.enabled = config.enabled;
    guard.tasks = config
        .tasks
        .iter()
        .map(|task| {
            let mut schedule = task.schedule.clone();
            schedule.execution_mode = task.execution_mode.clone().or(schedule.execution_mode);
            schedule.confirm_timeout_minutes = task
                .confirm_timeout_minutes
                .or(schedule.confirm_timeout_minutes);
            let schedule = normalize_schedule(schedule);
            WakeupTask {
                id: task.id.clone(),
                name: task.name.clone(),
                enabled: task.enabled,
                last_run_at: task.last_run_at,
                execution_mode: schedule.execution_mode.clone(),
                confirm_timeout_minutes: schedule.confirm_timeout_minutes,
                schedule,
            }
        })
        .collect();
    guard.last_executed_at.clear();
    guard.last_claim_at.clear();
    let ids: HashSet<_> = config.tasks.iter().map(|task| task.id.as_str()).collect();
    guard.reset_states.retain(|id, _| ids.contains(id.as_str()));
    for (id, row) in runtime {
        if let Some(last_run) = row.last_run_at {
            guard.last_executed_at.insert(id.clone(), last_run);
        }
        if let Some(last_claim) = row.last_claim_at {
            guard.last_claim_at.insert(id.clone(), last_claim);
        }
        let previous = guard.reset_states.entry(id).or_default();
        previous.last_reset_trigger_timestamps = row.reset.last_reset_trigger_timestamps;
        previous.last_reset_trigger_at = row.reset.last_reset_trigger_at;
        previous
            .last_reset_remaining
            .extend(row.reset.last_reset_remaining);
    }
    lock_or_recover(pending_confirmations(), "pending confirmations lock")
        .retain(|_, pending| task_rule_is_current(&guard, &pending.task));
}

fn task_rule_is_current(state: &SchedulerState, expected: &WakeupTask) -> bool {
    state.enabled
        && state.tasks.iter().any(|task| {
            task.id == expected.id && task.enabled && task.schedule == expected.schedule
        })
}

pub fn load_state() -> Result<PersistedWakeupState, String> {
    let _guard = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
    Ok(wakeup_store()?.load()?.0.unwrap_or_default())
}

fn apply_saved_state(store: &store::WakeupStore) -> Result<PersistedWakeupState, String> {
    let (config, runtime) = store.load()?;
    let config = config.unwrap_or_default();
    apply_state(&config, runtime);
    Ok(config)
}

pub fn restore_state_from_disk() {
    let _guard = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
    if let Err(error) = wakeup_store().and_then(|store| apply_saved_state(&store)) {
        modules::logger::log_warn(&format!("[WakeupTasks] 读取持久化任务失败: {error}"));
    }
}

pub fn initialize_state(
    legacy: Option<PersistedWakeupState>,
) -> Result<PersistedWakeupState, String> {
    let _guard = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
    let store = wakeup_store()?;
    if let Some(legacy) = legacy {
        seed_webview(&store, legacy)?;
    }
    apply_saved_state(&store)
}

fn seed_webview(store: &store::WakeupStore, legacy: PersistedWakeupState) -> Result<(), String> {
    let current = store.load()?.0;
    match current {
        Some(mut config) if !config.webview_seeded => {
            let old: HashMap<_, _> = legacy
                .tasks
                .iter()
                .map(|task| (task.id.as_str(), task))
                .collect();
            for task in &mut config.tasks {
                if let Some(old) = old.get(task.id.as_str()) {
                    if task.created_at == 0 {
                        task.created_at = old.created_at;
                    }
                    if task.execution_mode.is_none() && task.schedule.execution_mode.is_none() {
                        task.execution_mode = old
                            .execution_mode
                            .clone()
                            .or_else(|| old.schedule.execution_mode.clone());
                    }
                    if task.confirm_timeout_minutes.is_none()
                        && task.schedule.confirm_timeout_minutes.is_none()
                    {
                        task.confirm_timeout_minutes = old
                            .confirm_timeout_minutes
                            .or(old.schedule.confirm_timeout_minutes);
                    }
                    if old.last_run_at > task.last_run_at {
                        task.last_run_at = old.last_run_at;
                        store.seed_latest_run(task)?;
                    }
                }
            }
            config.webview_seeded = true;
            store.save_rules(&config, &[])?;
        }
        None => {
            let config = PersistedWakeupState {
                webview_seeded: true,
                ..legacy
            };
            store.save_rules(&config, &config.tasks.iter().collect::<Vec<_>>())?;
        }
        Some(_) => {}
    }
    Ok(())
}

fn normalize_schedule(raw: ScheduleConfig) -> ScheduleConfigNormalized {
    let daily_times = raw
        .daily_times
        .filter(|times| !times.is_empty())
        .unwrap_or_else(|| vec!["08:00".to_string()]);
    let weekly_days = raw
        .weekly_days
        .filter(|days| !days.is_empty())
        .unwrap_or_else(|| vec![1, 2, 3, 4, 5]);
    let weekly_times = raw
        .weekly_times
        .filter(|times| !times.is_empty())
        .unwrap_or_else(|| vec!["08:00".to_string()]);
    let interval_hours = raw.interval_hours.unwrap_or(4).max(1);
    let interval_start_time = raw
        .interval_start_time
        .unwrap_or_else(|| "07:00".to_string());
    let interval_end_time = raw.interval_end_time.unwrap_or_else(|| "22:00".to_string());
    let max_output_tokens = raw.max_output_tokens.unwrap_or(0).max(0);
    let fallback_times = raw
        .fallback_times
        .filter(|times| !times.is_empty())
        .unwrap_or_else(|| vec!["07:00".to_string()]);
    let startup_delay_minutes = raw
        .startup_delay_minutes
        .map(|value| value.clamp(0, 24 * 60));
    let execution_mode = raw
        .execution_mode
        .filter(|mode| mode == "auto" || mode == "confirm")
        .unwrap_or_else(|| "auto".to_string());
    let confirm_timeout_minutes = raw
        .confirm_timeout_minutes
        .filter(|&v| v >= 1 && v <= 60)
        .unwrap_or(5);
    ScheduleConfigNormalized {
        repeat_mode: raw.repeat_mode,
        daily_times,
        weekly_days,
        weekly_times,
        interval_hours,
        interval_start_time,
        interval_end_time,
        selected_models: raw.selected_models,
        selected_accounts: raw.selected_accounts,
        crontab: raw.crontab,
        wake_on_reset: raw.wake_on_reset.unwrap_or(false),
        custom_prompt: raw.custom_prompt,
        max_output_tokens,
        transport_mode: raw.transport_mode,
        time_window_enabled: raw.time_window_enabled.unwrap_or(false),
        time_window_start: raw.time_window_start,
        time_window_end: raw.time_window_end,
        fallback_times,
        startup_delay_minutes,
        execution_mode,
        confirm_timeout_minutes,
    }
}

pub fn sync_state(
    enabled: bool,
    tasks: Vec<WakeupTaskInput>,
    expected_rules_revision: Option<u64>,
) -> Result<PersistedWakeupState, String> {
    let _guard = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
    let store = wakeup_store()?;
    let current = store.load()?.0.unwrap_or_default();
    if expected_rules_revision.is_some_and(|revision| revision != current.rules_revision) {
        return Err("WAKEUP_RULES_CHANGED".into());
    }
    let existing: HashSet<_> = current.tasks.iter().map(|task| task.id.as_str()).collect();
    let config = PersistedWakeupState {
        enabled,
        tasks,
        webview_seeded: true,
        rules_revision: current.rules_revision.saturating_add(1),
    };
    let new_tasks: Vec<_> = config
        .tasks
        .iter()
        .filter(|task| !existing.contains(task.id.as_str()))
        .collect();
    store.save_rules(&config, &new_tasks)?;
    apply_saved_state(&store)
}

pub fn trigger_startup_tasks_if_needed(app: AppHandle) {
    let has_startup_tasks = {
        let guard = lock_or_recover(state(), "wakeup state lock");
        guard.enabled
            && guard
                .tasks
                .iter()
                .any(|task| task.enabled && task.schedule.startup_delay_minutes.is_some())
    };
    if !has_startup_tasks {
        return;
    }

    let should_trigger = {
        let mut startup_triggered =
            lock_or_recover(startup_triggered_flag(), "wakeup startup trigger lock");
        if *startup_triggered {
            false
        } else {
            *startup_triggered = true;
            true
        }
    };
    if !should_trigger {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let started = run_enabled_tasks_now(&app, "startup").await;
        if started > 0 {
            modules::logger::log_info(&format!(
                "[WakeupTasks] 应用启动触发自启任务: started={}",
                started
            ));
        }
    });
}

pub fn ensure_started(app: AppHandle) {
    let mut started = lock_or_recover(started_flag(), "wakeup started lock");
    if *started {
        return;
    }
    *started = true;

    tauri::async_runtime::spawn(async move {
        loop {
            run_scheduler_once(&app).await;
            // 检查确认超时，避免因前端未调用导致确认任务堆积
            if let Err(e) = check_and_handle_timeouts(&app).await {
                modules::logger::log_warn(&format!("[WakeupTasks] 超时检查失败: {}", e));
            }
            sleep(Duration::from_secs(30)).await;
        }
    });
}

pub async fn run_enabled_tasks_now(app: &AppHandle, trigger_source: &str) -> usize {
    let snapshot = {
        let guard = lock_or_recover(state(), "wakeup state lock");
        guard.clone()
    };

    if !snapshot.enabled {
        return 0;
    }

    let source = {
        let trimmed = trigger_source.trim();
        if trimmed.is_empty() {
            "startup"
        } else {
            trimmed
        }
    };

    if source == "startup" {
        let startup_tasks: Vec<(String, i32)> = snapshot
            .tasks
            .iter()
            .filter(|task| task.enabled)
            .filter_map(|task| {
                task.schedule
                    .startup_delay_minutes
                    .map(|delay| (task.id.clone(), delay.max(0)))
            })
            .collect();

        for (task_id, delay_minutes) in startup_tasks.iter() {
            let app_handle = app.clone();
            let task_id = task_id.clone();
            let delay_seconds = (*delay_minutes as u64) * 60;
            tauri::async_runtime::spawn(async move {
                if delay_seconds > 0 {
                    sleep(Duration::from_secs(delay_seconds)).await;
                }
                if let Some(task) = resolve_startup_task_to_run(&task_id) {
                    run_task(&app_handle, &task, "startup", None).await;
                }
            });
        }
        return startup_tasks.len();
    }

    let mut started_count = 0usize;
    for task in snapshot.tasks.iter() {
        if !task.enabled || task.schedule.startup_delay_minutes.is_some() {
            continue;
        }
        let running = {
            let guard = lock_or_recover(state(), "wakeup state lock");
            guard.running_tasks.contains(&task.id)
        };
        if running {
            continue;
        }
        run_task(app, task, source, None).await;
        started_count += 1;
    }

    started_count
}

fn resolve_startup_task_to_run(task_id: &str) -> Option<WakeupTask> {
    let guard = lock_or_recover(state(), "wakeup state lock");
    if !guard.enabled || guard.running_tasks.contains(task_id) {
        return None;
    }
    guard
        .tasks
        .iter()
        .find(|task| {
            task.id == task_id && task.enabled && task.schedule.startup_delay_minutes.is_some()
        })
        .cloned()
}

fn parse_time_to_minutes(value: &str) -> Option<i32> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: i32 = parts[0].parse().ok()?;
    let m: i32 = parts[1].parse().ok()?;
    if h < 0 || h > 23 || m < 0 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

fn is_in_time_window(start: Option<&String>, end: Option<&String>, now: DateTime<Local>) -> bool {
    let Some(start) = start else {
        return true;
    };
    let Some(end) = end else {
        return true;
    };
    let Some(start_minutes) = parse_time_to_minutes(start) else {
        return true;
    };
    let Some(end_minutes) = parse_time_to_minutes(end) else {
        return true;
    };
    let current_minutes = (now.hour() as i32) * 60 + now.minute() as i32;

    if start_minutes <= end_minutes {
        current_minutes >= start_minutes && current_minutes < end_minutes
    } else {
        current_minutes >= start_minutes || current_minutes < end_minutes
    }
}

fn next_run_time(
    schedule: &ScheduleConfigNormalized,
    after: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let mut results: Vec<DateTime<Local>> = Vec::new();
    if schedule.repeat_mode == "daily" && !schedule.daily_times.is_empty() {
        let mut times = schedule.daily_times.clone();
        times.sort();
        for day_offset in 0..7 {
            for time in &times {
                if let Some(candidate) = build_datetime(after, day_offset, &time) {
                    if candidate > after {
                        results.push(candidate);
                        if !results.is_empty() {
                            return results.into_iter().min();
                        }
                    }
                }
            }
        }
    } else if schedule.repeat_mode == "weekly"
        && !schedule.weekly_days.is_empty()
        && !schedule.weekly_times.is_empty()
    {
        let mut times = schedule.weekly_times.clone();
        times.sort();
        for day_offset in 0..14 {
            let date = after + chrono::Duration::days(day_offset);
            let weekday = date.weekday().num_days_from_sunday() as i32;
            if schedule.weekly_days.contains(&weekday) {
                for time in &times {
                    if let Some(candidate) = build_datetime_from_date(date, &time) {
                        if candidate > after {
                            results.push(candidate);
                            if !results.is_empty() {
                                return results.into_iter().min();
                            }
                        }
                    }
                }
            }
        }
    } else if schedule.repeat_mode == "interval" {
        let start_minutes = parse_time_to_minutes(&schedule.interval_start_time).unwrap_or(7 * 60);
        let end_minutes = parse_time_to_minutes(&schedule.interval_end_time).unwrap_or(22 * 60);
        let interval = schedule.interval_hours.max(1);

        for day_offset in 0..7 {
            let base_date = after + chrono::Duration::days(day_offset);
            let Some(window_start) = build_datetime_from_minutes(base_date, start_minutes) else {
                continue;
            };
            let Some(mut window_end) = build_datetime_from_minutes(base_date, end_minutes) else {
                continue;
            };
            if start_minutes > end_minutes {
                window_end += chrono::Duration::days(1);
            }

            let mut candidate = window_start;
            while candidate <= window_end {
                if candidate > after {
                    results.push(candidate);
                    if !results.is_empty() {
                        return results.into_iter().min();
                    }
                }
                candidate += chrono::Duration::hours(interval as i64);
            }
        }
    }
    None
}

fn build_datetime(base: DateTime<Local>, day_offset: i64, time: &str) -> Option<DateTime<Local>> {
    let date = base + chrono::Duration::days(day_offset);
    build_datetime_from_date(date, time)
}

fn build_datetime_from_date(date: DateTime<Local>, time: &str) -> Option<DateTime<Local>> {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let naive_date = date.date_naive();
    let naive = naive_date.and_hms_opt(h, m, 0)?;
    Local.from_local_datetime(&naive).single()
}

fn build_datetime_from_minutes(date: DateTime<Local>, minutes: i32) -> Option<DateTime<Local>> {
    let normalized = minutes.rem_euclid(24 * 60);
    let h = (normalized / 60) as u32;
    let m = (normalized % 60) as u32;
    let naive_date = date.date_naive();
    let naive = naive_date.and_hms_opt(h, m, 0)?;
    Local.from_local_datetime(&naive).single()
}

#[derive(Debug, Clone)]
struct CronField {
    values: HashSet<i32>,
    wildcard: bool,
}

impl CronField {
    fn contains(&self, value: i32) -> bool {
        self.values.contains(&value)
    }
}

#[derive(Debug, Clone)]
struct ParsedCrontab {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

fn next_crontab_time(expr: &str, after: DateTime<Local>) -> Option<DateTime<Local>> {
    let parsed = parse_crontab_expression(expr).ok()?;
    let mut candidate = (after + chrono::Duration::minutes(1))
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))?;

    const MAX_LOOKAHEAD_MINUTES: i64 = 366 * 24 * 60;
    for _ in 0..MAX_LOOKAHEAD_MINUTES {
        if crontab_matches_time(&parsed, candidate) {
            return Some(candidate);
        }
        candidate += chrono::Duration::minutes(1);
    }
    None
}

fn crontab_matches_time(expr: &ParsedCrontab, candidate: DateTime<Local>) -> bool {
    let minute = candidate.minute() as i32;
    let hour = candidate.hour() as i32;
    let day_of_month = candidate.day() as i32;
    let month = candidate.month() as i32;
    let day_of_week = candidate.weekday().num_days_from_sunday() as i32;

    if !expr.minute.contains(minute) || !expr.hour.contains(hour) || !expr.month.contains(month) {
        return false;
    }

    let day_of_month_match = expr.day_of_month.contains(day_of_month);
    let day_of_week_match = expr.day_of_week.contains(day_of_week);
    let day_match = if expr.day_of_month.wildcard && expr.day_of_week.wildcard {
        true
    } else if expr.day_of_month.wildcard {
        day_of_week_match
    } else if expr.day_of_week.wildcard {
        day_of_month_match
    } else {
        day_of_month_match || day_of_week_match
    };

    day_match
}

pub fn validate_crontab_expression(expr: &str) -> Result<(), String> {
    parse_crontab_expression(expr).map(|_| ())
}

fn parse_crontab_expression(expr: &str) -> Result<ParsedCrontab, String> {
    let parts: Vec<&str> = expr.trim().split_whitespace().collect();
    if parts.len() != 5 {
        return Err("crontab 表达式必须是 5 段（分 时 日 月 周）".to_string());
    }

    Ok(ParsedCrontab {
        minute: parse_cron_field(parts[0], 0, 59, false, "分钟")?,
        hour: parse_cron_field(parts[1], 0, 23, false, "小时")?,
        day_of_month: parse_cron_field(parts[2], 1, 31, false, "日期")?,
        month: parse_cron_field(parts[3], 1, 12, false, "月份")?,
        day_of_week: parse_cron_field(parts[4], 0, 7, true, "星期")?,
    })
}

fn parse_cron_field(
    field: &str,
    min: i32,
    max: i32,
    normalize_weekday: bool,
    field_label: &str,
) -> Result<CronField, String> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return Err(format!("crontab {}字段不能为空", field_label));
    }

    if trimmed == "*" {
        let mut values = HashSet::new();
        for value in min..=max {
            values.insert(normalize_cron_value(value, normalize_weekday));
        }
        return Ok(CronField {
            values,
            wildcard: true,
        });
    }

    let mut values = HashSet::new();
    for segment in trimmed.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            return Err(format!("crontab {}字段包含空片段", field_label));
        }
        parse_cron_segment(
            segment,
            min,
            max,
            normalize_weekday,
            field_label,
            &mut values,
        )?;
    }

    if values.is_empty() {
        return Err(format!("crontab {}字段没有可用取值", field_label));
    }

    Ok(CronField {
        values,
        wildcard: false,
    })
}

fn parse_cron_segment(
    segment: &str,
    min: i32,
    max: i32,
    normalize_weekday: bool,
    field_label: &str,
    out: &mut HashSet<i32>,
) -> Result<(), String> {
    let (range_part, step) = if let Some((left, right)) = segment.split_once('/') {
        let step = parse_cron_number(right, field_label, "步长")?;
        if step <= 0 {
            return Err(format!("crontab {}字段步长必须大于 0", field_label));
        }
        (left.trim(), step)
    } else {
        (segment, 1)
    };

    if range_part == "*" {
        insert_cron_range(out, min, max, step, normalize_weekday);
        return Ok(());
    }

    if let Some((start_raw, end_raw)) = range_part.split_once('-') {
        let start = parse_cron_number(start_raw, field_label, "范围起点")?;
        let end = parse_cron_number(end_raw, field_label, "范围终点")?;
        validate_cron_value(start, min, max, normalize_weekday, field_label)?;
        validate_cron_value(end, min, max, normalize_weekday, field_label)?;
        if end < start {
            return Err(format!(
                "crontab {}字段范围无效：{}-{}",
                field_label, start, end
            ));
        }
        insert_cron_range(out, start, end, step, normalize_weekday);
        return Ok(());
    }

    let single = parse_cron_number(range_part, field_label, "取值")?;
    validate_cron_value(single, min, max, normalize_weekday, field_label)?;
    if step == 1 {
        out.insert(normalize_cron_value(single, normalize_weekday));
    } else {
        insert_cron_range(out, single, max, step, normalize_weekday);
    }
    Ok(())
}

fn parse_cron_number(raw: &str, field_label: &str, value_label: &str) -> Result<i32, String> {
    raw.trim().parse::<i32>().map_err(|_| {
        format!(
            "crontab {}字段{}无效：{}",
            field_label,
            value_label,
            raw.trim()
        )
    })
}

fn validate_cron_value(
    value: i32,
    min: i32,
    max: i32,
    normalize_weekday: bool,
    field_label: &str,
) -> Result<(), String> {
    if normalize_weekday && value == 7 {
        return Ok(());
    }
    if value < min || value > max {
        return Err(format!(
            "crontab {}字段取值超出范围（{}-{}）：{}",
            field_label, min, max, value
        ));
    }
    Ok(())
}

fn normalize_cron_value(value: i32, normalize_weekday: bool) -> i32 {
    if normalize_weekday && value == 7 {
        0
    } else {
        value
    }
}

fn insert_cron_range(
    out: &mut HashSet<i32>,
    start: i32,
    end: i32,
    step: i32,
    normalize_weekday: bool,
) {
    let mut value = start;
    while value <= end {
        out.insert(normalize_cron_value(value, normalize_weekday));
        value += step;
    }
}

fn normalize_max_tokens(value: i32) -> u32 {
    if value > 0 {
        value as u32
    } else {
        0
    }
}

fn should_trigger_on_reset(
    state: &mut ResetState,
    model_key: &str,
    reset_at: &str,
    remaining_percent: i32,
) -> bool {
    if remaining_percent < 100 {
        state
            .last_reset_remaining
            .insert(model_key.to_string(), remaining_percent);
        return false;
    }

    let now = chrono::Utc::now().timestamp_millis();
    if let Some(last_reset_at) = state.last_reset_trigger_timestamps.get(model_key) {
        if let Ok(last_reset_time) =
            DateTime::parse_from_rfc3339(last_reset_at).map(|dt| dt.timestamp_millis())
        {
            let safe_time = last_reset_time + RESET_SAFETY_MARGIN_MS;
            if now < safe_time {
                state
                    .last_reset_remaining
                    .insert(model_key.to_string(), remaining_percent);
                return false;
            }
        }
    }

    if let Some(last_trigger_at) = state.last_reset_trigger_at.get(model_key) {
        if now - *last_trigger_at < RESET_TRIGGER_COOLDOWN_MS {
            state
                .last_reset_remaining
                .insert(model_key.to_string(), remaining_percent);
            return false;
        }
    }

    if state.last_reset_trigger_timestamps.get(model_key) == Some(&reset_at.to_string()) {
        state
            .last_reset_remaining
            .insert(model_key.to_string(), remaining_percent);
        return false;
    }

    state
        .last_reset_remaining
        .insert(model_key.to_string(), remaining_percent);
    true
}

fn mark_reset_triggered(state: &mut ResetState, model_key: &str, reset_at: &str) {
    state
        .last_reset_trigger_timestamps
        .insert(model_key.to_string(), reset_at.to_string());
    state
        .last_reset_trigger_at
        .insert(model_key.to_string(), chrono::Utc::now().timestamp_millis());
}

async fn run_scheduler_once(app: &AppHandle) {
    let snapshot = {
        let guard = lock_or_recover(state(), "wakeup state lock");
        guard.clone()
    };

    if !snapshot.enabled {
        return;
    }

    // 平台在「平台布局」中被禁用时不执行唤醒任务
    if !crate::modules::tray_layout::is_platform_enabled("antigravity") {
        return;
    }

    let now = Local::now();

    for task in snapshot.tasks.iter() {
        if !task.enabled {
            continue;
        }
        if snapshot.running_tasks.contains(&task.id) {
            continue;
        }
        if task.schedule.startup_delay_minutes.is_some() {
            continue;
        }

        if task.schedule.wake_on_reset {
            handle_quota_reset_task(app, task, now);
            continue;
        }

        // 优先使用本地记录的执行时间，避免被前端同步覆盖导致重复执行
        let local_last_run = snapshot.last_executed_at.get(&task.id).copied();
        let after = local_last_run
            .max(task.last_run_at)
            .max(snapshot.last_claim_at.get(&task.id).copied())
            .and_then(|ts| Local.timestamp_millis_opt(ts).single())
            .unwrap_or_else(|| now - chrono::Duration::minutes(1));

        let next_run = if let Some(expr) = &task.schedule.crontab {
            next_crontab_time(expr, after)
        } else {
            next_run_time(&task.schedule, after)
        };

        // 只有到达预定时间才触发（不再提前30秒）
        if let Some(next_run) = next_run {
            if next_run <= now {
                let trigger_source = if task.schedule.crontab.is_some() {
                    "crontab"
                } else {
                    "scheduled"
                };
                let app_handle = app.clone();
                let task_clone = task.clone();
                let trigger_source = trigger_source.to_string();
                tauri::async_runtime::spawn(async move {
                    run_task(
                        &app_handle,
                        &task_clone,
                        &trigger_source,
                        Some(next_run.timestamp_millis()),
                    )
                    .await;
                });
            }
        }
    }
}

fn handle_quota_reset_task(app: &AppHandle, task: &WakeupTask, now: DateTime<Local>) {
    if task.schedule.time_window_enabled {
        let in_window = is_in_time_window(
            task.schedule.time_window_start.as_ref(),
            task.schedule.time_window_end.as_ref(),
            now,
        );
        let in_fallback_time = task.schedule.fallback_times.iter().any(|time| {
            parse_time_to_minutes(time) == Some((now.hour() as i32) * 60 + now.minute() as i32)
        });
        if !in_window && !in_fallback_time {
            return;
        }
    }

    let accounts = match modules::list_accounts() {
        Ok(list) => list,
        Err(_) => return,
    };

    let selected_accounts: Vec<_> = task
        .schedule
        .selected_accounts
        .iter()
        .filter_map(|email| {
            accounts
                .iter()
                .find(|acc| acc.email.eq_ignore_ascii_case(email))
        })
        .collect();

    if selected_accounts.is_empty() {
        return;
    }

    let (models_to_trigger, candidate) = {
        let mut guard = lock_or_recover(state(), "wakeup state lock");
        let mut reset_state = guard
            .reset_states
            .get(&task.id)
            .cloned()
            .unwrap_or_default();

        let mut models_to_trigger: HashSet<String> = HashSet::new();
        for model_id in &task.schedule.selected_models {
            for account in &selected_accounts {
                let quota_models = account
                    .quota
                    .as_ref()
                    .map(|q| q.models.as_slice())
                    .unwrap_or(&[]);
                if let Some(quota) = quota_models.iter().find(|item| item.name == *model_id) {
                    if should_trigger_on_reset(
                        &mut reset_state,
                        model_id,
                        &quota.reset_time,
                        quota.percentage,
                    ) {
                        models_to_trigger.insert(model_id.clone());
                        mark_reset_triggered(&mut reset_state, model_id, &quota.reset_time);
                    }
                }
            }
        }
        if models_to_trigger.is_empty() {
            guard
                .reset_states
                .insert(task.id.clone(), reset_state.clone());
        }
        (models_to_trigger, reset_state)
    };

    if !models_to_trigger.is_empty() {
        let app_handle = app.clone();
        let task_clone = task.clone();
        let models = models_to_trigger.into_iter().collect::<Vec<_>>();
        tauri::async_runtime::spawn(async move {
            run_task_with_models(
                &app_handle,
                &task_clone,
                "quota_reset",
                models,
                None,
                Some(candidate),
            )
            .await;
        });
    }
}

async fn run_task(app: &AppHandle, task: &WakeupTask, trigger_source: &str, due_at: Option<i64>) {
    // 检查是否需要确认
    if task.execution_mode == "confirm" {
        let timeout_minutes = task.confirm_timeout_minutes;
        let timeout_at = chrono::Local::now().timestamp() + (timeout_minutes as i64 * 60);

        let pending = PendingConfirmation {
            task: task.clone(),
            trigger_source: trigger_source.to_string(),
            scheduled_at: chrono::Local::now().timestamp(),
            timeout_at,
            due_at,
        };

        {
            let state = lock_or_recover(state(), "wakeup state lock");
            if !task_rule_is_current(&state, task) {
                return;
            }
            let mut queue = lock_or_recover(pending_confirmations(), "pending confirmations lock");
            if queue.contains_key(&task.id) {
                return;
            }
            queue.insert(task.id.clone(), pending);
        }

        // 发送通知
        let notification_id = send_confirmation_notification(app, &task.name, &task.schedule).await;

        // 存储到待确认队列并发射事件
        emit_confirmation_mapping(app, task.id.clone(), notification_id);

        modules::logger::log_info(&format!(
            "[WakeupTasks] 任务 {} 需要确认，已发送通知",
            task.name
        ));
        return;
    }

    // 直接执行模式（现有逻辑）
    run_task_with_models(
        app,
        task,
        trigger_source,
        task.schedule.selected_models.clone(),
        due_at,
        None,
    )
    .await;
}

async fn send_confirmation_notification(
    app: &AppHandle,
    task_name: &str,
    schedule: &ScheduleConfigNormalized,
) -> u32 {
    let account_emails: Vec<String> = schedule.selected_accounts.clone();
    let models: Vec<String> = schedule.selected_models.clone();

    let body = format!(
        "任务: {}\n账号: {}\n模型: {}\n点击确认执行唤醒",
        task_name,
        account_emails.join(", "),
        models.join(", ")
    );

    match app
        .notification()
        .builder()
        .title("唤醒任务待确认")
        .body(&body)
        .show()
    {
        Ok(()) => 0,
        Err(e) => {
            modules::logger::log_warn(&format!("[WakeupTasks] 发送通知失败: {}", e));
            0
        }
    }
}

fn emit_confirmation_mapping(app: &AppHandle, task_id: String, notification_id: u32) {
    // 发射事件通知前端建立映射
    #[derive(serde::Serialize, Clone)]
    struct NotificationMappingPayload {
        task_id: String,
        notification_id: u32,
    }

    let payload = NotificationMappingPayload {
        task_id,
        notification_id,
    };

    if let Err(e) = app.emit("wakeup://notification-mapping", payload) {
        modules::logger::log_warn(&format!("[WakeupTasks] 发射通知映射事件失败: {}", e));
    }
}

pub async fn execute_pending_confirmation(app: &AppHandle, task_id: &str) -> Result<(), String> {
    let pending = {
        let mut lock = lock_or_recover(pending_confirmations(), "pending confirmations lock");
        lock.remove(task_id)
    };

    if let Some(pending) = pending {
        // 检查是否超时
        if chrono::Local::now().timestamp() > pending.timeout_at {
            record_task_history(
                app,
                &pending.task,
                &pending.trigger_source,
                "skipped_timeout",
            )
            .await;
            return Ok(());
        }

        // 执行唤醒
        run_task_with_models(
            app,
            &pending.task,
            &pending.trigger_source,
            pending.task.schedule.selected_models.clone(),
            pending.due_at,
            None,
        )
        .await;
    }

    Ok(())
}

pub fn cancel_pending_confirmation(task_id: &str) -> Result<(), String> {
    let mut lock = lock_or_recover(pending_confirmations(), "pending confirmations lock");
    lock.remove(task_id);
    Ok(())
}

pub async fn check_and_handle_timeouts(app: &AppHandle) -> Result<(), String> {
    let timed_out_tasks: Vec<(String, PendingConfirmation)> = {
        let mut lock = lock_or_recover(pending_confirmations(), "pending confirmations lock");
        let now = chrono::Local::now().timestamp();
        let expired_ids: Vec<String> = lock
            .iter()
            .filter(|(_, pending)| now > pending.timeout_at)
            .map(|(id, _)| id.clone())
            .collect();

        expired_ids
            .into_iter()
            .filter_map(|id| lock.remove(&id).map(|pending| (id, pending)))
            .collect()
    };

    for (_task_id, pending) in timed_out_tasks {
        record_task_history(
            app,
            &pending.task,
            &pending.trigger_source,
            "skipped_timeout",
        )
        .await;
    }

    Ok(())
}

async fn record_task_history(
    _app: &AppHandle,
    task: &WakeupTask,
    trigger_source: &str,
    status: &str,
) {
    let task = task.clone();
    let trigger_source = trigger_source.to_string();
    let status = status.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_history::record_status(&task, &trigger_source, &status)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = result {
        modules::logger::log_warn(&format!("[WakeupTasks] 记录历史失败: {error}"));
    }
}

async fn run_task_with_models(
    app: &AppHandle,
    task: &WakeupTask,
    trigger_source: &str,
    mut models: Vec<String>,
    due_at: Option<i64>,
    reset: Option<ResetState>,
) {
    if models.is_empty() {
        return;
    }

    let accounts = match modules::list_accounts() {
        Ok(list) => list,
        Err(_) => return,
    };

    let selected_accounts: Vec<_> = task
        .schedule
        .selected_accounts
        .iter()
        .filter_map(|email| {
            accounts
                .iter()
                .find(|acc| acc.email.eq_ignore_ascii_case(email))
        })
        .collect();

    if selected_accounts.is_empty() {
        return;
    }

    {
        let mut guard = lock_or_recover(state(), "wakeup state lock");
        if !guard.running_tasks.insert(task.id.clone()) {
            return;
        }
    }
    struct Running(String);
    impl Drop for Running {
        fn drop(&mut self) {
            lock_or_recover(state(), "wakeup state lock")
                .running_tasks
                .remove(&self.0);
        }
    }
    let _running = Running(task.id.clone());

    let expected = task.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _rules = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
        {
            let guard = lock_or_recover(state(), "wakeup state lock");
            if !task_rule_is_current(&guard, &expected) {
                return Ok(None);
            }
        }
        let claimed_at = chrono::Utc::now().timestamp_millis();
        let claimed = wakeup_store()?.claim(
            &expected.id,
            expected.last_run_at,
            due_at,
            claimed_at,
            reset.as_ref(),
            &mut models,
        )?;
        if !claimed {
            return Ok(None);
        }
        let mut guard = lock_or_recover(state(), "wakeup state lock");
        guard.last_claim_at.insert(expected.id.clone(), claimed_at);
        if let Some(reset) = reset {
            guard.reset_states.insert(expected.id, reset);
        }
        Ok::<_, String>(Some((claimed_at, models)))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let (claimed_at, models) = match result {
        Ok(Some(run)) => run,
        Ok(None) => return,
        Err(error) => {
            modules::logger::log_warn(&format!("[WakeupTasks] 唤醒任务认领失败: {error}"));
            return;
        }
    };

    let prompt = task
        .schedule
        .custom_prompt
        .as_ref()
        .and_then(|p| {
            if p.trim().is_empty() {
                None
            } else {
                Some(p.trim().to_string())
            }
        })
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let max_tokens = normalize_max_tokens(task.schedule.max_output_tokens);

    let mut history: Vec<modules::wakeup_history::WakeupHistoryItem> = Vec::new();
    let mut successful_accounts = HashSet::new();
    for account in &selected_accounts {
        for model in &models {
            let started = chrono::Utc::now();
            let result = modules::wakeup::trigger_wakeup(
                &account.id,
                model,
                &prompt,
                max_tokens,
                None,
                task.schedule.transport_mode,
            )
            .await;
            let duration = chrono::Utc::now()
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0) as u64;
            let (success, message) = match result {
                Ok(resp) => {
                    successful_accounts.insert(account.id.clone());
                    // 唤醒成功，账号可正常发起请求，解除所有类型的禁用
                    if let Ok(mut acc) = modules::load_account(&account.id) {
                        if acc.disabled {
                            modules::logger::log_info(&format!(
                                "[WakeupScheduler] 唤醒成功，自动解除禁用状态: {}",
                                acc.email
                            ));
                            acc.clear_disabled();
                            acc.quota_error = None;
                            let _ = modules::save_account(&acc);
                        }
                    }
                    (true, Some(resp.reply))
                }
                Err(err) => (false, Some(err.to_string())),
            };
            history.push(modules::wakeup_history::WakeupHistoryItem {
                id: format!(
                    "{}-{}",
                    chrono::Utc::now().timestamp_millis(),
                    history.len()
                ),
                timestamp: chrono::Utc::now().timestamp_millis(),
                trigger_type: "auto".to_string(),
                trigger_source: trigger_source.to_string(),
                task_name: Some(task.name.clone()),
                account_email: account.email.clone(),
                model_id: model.clone(),
                prompt: Some(prompt.clone()),
                success,
                status: if success {
                    Some("success".to_string())
                } else {
                    Some("failed".to_string())
                },
                message,
                duration: Some(duration),
            });
        }
    }

    let executed_at = chrono::Utc::now().timestamp_millis();
    let id = task.id.clone();
    let completion = tauri::async_runtime::spawn_blocking(move || {
        let _rules = lock_or_recover(&RULES_LOCK, "wakeup rules lock");
        if wakeup_store()?.finish(&id, claimed_at, executed_at)? {
            let mut guard = lock_or_recover(state(), "wakeup state lock");
            if let Some(item) = guard.tasks.iter_mut().find(|item| item.id == id) {
                item.last_run_at = Some(executed_at);
                guard.last_executed_at.insert(id, executed_at);
                return Ok(true);
            }
        }
        Ok::<_, String>(false)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let persisted = completion.unwrap_or_else(|error| {
        modules::logger::log_warn(&format!("[WakeupTasks] 保存唤醒任务结果失败: {error}"));
        false
    });

    let persisted_history = history.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_history::add_history_items(persisted_history)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = result {
        modules::logger::log_error(&format!("写入唤醒历史失败: {error}"));
    }

    let payload = WakeupTaskResultPayload {
        task_id: task.id.clone(),
        last_run_at: executed_at,
        persisted,
        records: history,
    };
    let _ = app.emit("wakeup://task-result", payload);
    quota_refresh::schedule(app, successful_accounts);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WakeupTaskResultPayload {
    task_id: String,
    last_run_at: i64,
    persisted: bool,
    records: Vec<modules::wakeup_history::WakeupHistoryItem>,
}

// (no local helpers)
