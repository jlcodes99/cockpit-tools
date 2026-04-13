use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::time::sleep;

use crate::modules;

const DEFAULT_PROMPT: &str = "hi";
const RESET_TRIGGER_COOLDOWN_MS: i64 = 10 * 60 * 1000;
const RESET_SAFETY_MARGIN_MS: i64 = 2 * 60 * 1000;
const WAKEUP_TASKS_FILE: &str = "wakeup_tasks.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupTaskInput {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub schedule: ScheduleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedWakeupState {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    tasks: Vec<WakeupTaskInput>,
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
    pub time_window_enabled: Option<bool>,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    pub fallback_times: Option<Vec<String>>,
    pub startup_delay_minutes: Option<i32>,
}

#[derive(Debug, Clone)]
struct WakeupTask {
    id: String,
    name: String,
    enabled: bool,
    last_run_at: Option<i64>,
    schedule: ScheduleConfigNormalized,
}

#[derive(Debug, Clone)]
struct ScheduleConfigNormalized {
    repeat_mode: String,
    daily_times: Vec<String>,
    weekly_days: Vec<i32>,
    weekly_times: Vec<String>,
    interval_hours: i32,
    interval_start_time: String,
    interval_end_time: String,
    selected_models: Vec<String>,
    selected_accounts: Vec<String>,
    crontab: Option<String>,
    wake_on_reset: bool,
    custom_prompt: Option<String>,
    max_output_tokens: i32,
    time_window_enabled: bool,
    time_window_start: Option<String>,
    time_window_end: Option<String>,
    fallback_times: Vec<String>,
    startup_delay_minutes: Option<i32>,
}

#[derive(Default, Debug, Clone)]
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
}

static STATE: OnceLock<Mutex<SchedulerState>> = OnceLock::new();
static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
static STARTUP_TRIGGERED: OnceLock<Mutex<bool>> = OnceLock::new();

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

fn tasks_state_path() -> Result<std::path::PathBuf, String> {
    Ok(modules::account::get_data_dir()?.join(WAKEUP_TASKS_FILE))
}

fn save_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位唤醒任务目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建唤醒任务目录失败: {}", e))?;
    let temp_path = parent.join(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("wakeup_tasks")
    ));
    let content =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化唤醒任务失败: {}", e))?;
    fs::write(&temp_path, content).map_err(|e| format!("写入唤醒任务临时文件失败: {}", e))?;
    fs::rename(&temp_path, path).map_err(|e| format!("替换唤醒任务文件失败: {}", e))
}

fn persist_state(enabled: bool, tasks: &[WakeupTaskInput]) -> Result<(), String> {
    let persisted = PersistedWakeupState {
        enabled,
        tasks: tasks.to_vec(),
    };
    save_json_atomic(&tasks_state_path()?, &persisted)
}

fn apply_state(enabled: bool, tasks: Vec<WakeupTaskInput>) {
    let mut guard = lock_or_recover(state(), "wakeup state lock");
    guard.enabled = enabled;
    guard.tasks = tasks
        .into_iter()
        .map(|task| WakeupTask {
            id: task.id,
            name: task.name,
            enabled: task.enabled,
            last_run_at: task.last_run_at,
            schedule: normalize_schedule(task.schedule),
        })
        .collect();
}

pub fn restore_state_from_disk() {
    let path = match tasks_state_path() {
        Ok(path) => path,
        Err(err) => {
            modules::logger::log_warn(&format!("[WakeupTasks] 获取持久化路径失败: {}", err));
            return;
        }
    };
    if !path.exists() {
        return;
    }
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            modules::logger::log_warn(&format!("[WakeupTasks] 读取持久化任务失败: {}", err));
            return;
        }
    };
    if content.trim().is_empty() {
        return;
    }
    let persisted: PersistedWakeupState = match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            modules::logger::log_warn(&format!("[WakeupTasks] 解析持久化任务失败: {}", err));
            return;
        }
    };
    apply_state(persisted.enabled, persisted.tasks);
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
        time_window_enabled: raw.time_window_enabled.unwrap_or(false),
        time_window_start: raw.time_window_start,
        time_window_end: raw.time_window_end,
        fallback_times,
        startup_delay_minutes,
    }
}

pub fn sync_state(enabled: bool, tasks: Vec<WakeupTaskInput>) {
    if let Err(err) = persist_state(enabled, &tasks) {
        modules::logger::log_warn(&format!("[WakeupTasks] 持久化任务状态失败: {}", err));
    }
    apply_state(enabled, tasks);
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
                    run_task(&app_handle, &task, "startup").await;
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
        run_task(app, task, source).await;
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
            .or(task.last_run_at)
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
                    run_task(&app_handle, &task_clone, &trigger_source).await;
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

    let models_to_trigger = {
        let mut state_guard = lock_or_recover(state(), "wakeup state lock");
        let reset_state = state_guard
            .reset_states
            .entry(task.id.clone())
            .or_insert_with(ResetState::default);

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
                        reset_state,
                        model_id,
                        &quota.reset_time,
                        quota.percentage,
                    ) {
                        models_to_trigger.insert(model_id.clone());
                        mark_reset_triggered(reset_state, model_id, &quota.reset_time);
                    }
                }
            }
        }
        models_to_trigger
    };

    if !models_to_trigger.is_empty() {
        let app_handle = app.clone();
        let task_clone = task.clone();
        let models = models_to_trigger.into_iter().collect::<Vec<_>>();
        tauri::async_runtime::spawn(async move {
            run_task_with_models(&app_handle, &task_clone, "quota_reset", models).await;
        });
    }
}

async fn run_task(app: &AppHandle, task: &WakeupTask, trigger_source: &str) {
    run_task_with_models(
        app,
        task,
        trigger_source,
        task.schedule.selected_models.clone(),
    )
    .await;
}

async fn run_task_with_models(
    app: &AppHandle,
    task: &WakeupTask,
    trigger_source: &str,
    models: Vec<String>,
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
        guard.running_tasks.insert(task.id.clone());
    }

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
    for account in &selected_accounts {
        for model in &models {
            let started = chrono::Utc::now();
            let result =
                modules::wakeup::trigger_wakeup(&account.id, model, &prompt, max_tokens, None)
                    .await;
            let duration = chrono::Utc::now()
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0) as u64;
            let (success, message) = match result {
                Ok(resp) => {
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
                message,
                duration: Some(duration),
            });
        }
    }

    {
        let mut guard = lock_or_recover(state(), "wakeup state lock");
        guard.running_tasks.remove(&task.id);
        let executed_at = chrono::Utc::now().timestamp_millis();
        guard.tasks.iter_mut().for_each(|item| {
            if item.id == task.id {
                item.last_run_at = Some(executed_at);
            }
        });
        // 记录本地执行时间，防止被前端同步覆盖导致重复执行
        guard.last_executed_at.insert(task.id.clone(), executed_at);
    }

    // 写入历史文件
    if let Err(e) = modules::wakeup_history::add_history_items(history.clone()) {
        modules::logger::log_error(&format!("写入唤醒历史失败: {}", e));
    }

    let payload = WakeupTaskResultPayload {
        task_id: task.id.clone(),
        last_run_at: chrono::Utc::now().timestamp_millis(),
        records: history,
    };
    let _ = app.emit("wakeup://task-result", payload);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WakeupTaskResultPayload {
    task_id: String,
    last_run_at: i64,
    records: Vec<modules::wakeup_history::WakeupHistoryItem>,
}

// (no local helpers)
