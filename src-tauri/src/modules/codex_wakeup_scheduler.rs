use crate::modules::{codex_account, codex_wakeup, logger};
use chrono::{DateTime, Datelike, Local, TimeZone};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;
use tokio::time::sleep;

static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
static RUNNING_TASKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static STARTUP_TRIGGERED: OnceLock<Mutex<bool>> = OnceLock::new();

fn started_flag() -> &'static Mutex<bool> {
    STARTED.get_or_init(|| Mutex::new(false))
}

fn running_tasks() -> &'static Mutex<HashSet<String>> {
    RUNNING_TASKS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn startup_triggered_flag() -> &'static Mutex<bool> {
    STARTUP_TRIGGERED.get_or_init(|| Mutex::new(false))
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, label: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(err) => {
            logger::log_warn(&format!(
                "[CodexWakeup] 检测到锁中毒，继续使用恢复数据: {}",
                label
            ));
            err.into_inner()
        }
    }
}

fn parse_time_to_minutes(value: &str) -> Option<i32> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour: i32 = parts[0].parse().ok()?;
    let minute: i32 = parts[1].parse().ok()?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return None;
    }
    Some(hour * 60 + minute)
}

fn build_local_datetime(date: chrono::NaiveDate, minutes: i32) -> Option<DateTime<Local>> {
    let hour = (minutes / 60) as u32;
    let minute = (minutes % 60) as u32;
    Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), hour, minute, 0)
        .earliest()
        .or_else(|| {
            Local
                .with_ymd_and_hms(date.year(), date.month(), date.day(), hour, minute, 0)
                .latest()
        })
}

fn collect_task_reset_timestamps(task: &codex_wakeup::CodexWakeupTask) -> Vec<i64> {
    if task.account_ids.is_empty() {
        return Vec::new();
    }
    let quota_reset_window = task
        .schedule
        .quota_reset_window
        .as_deref()
        .unwrap_or("either");
    let include_primary = quota_reset_window == "either" || quota_reset_window == "primary_window";
    let include_secondary =
        quota_reset_window == "either" || quota_reset_window == "secondary_window";

    let selected: HashSet<&str> = task.account_ids.iter().map(String::as_str).collect();
    let mut timestamps: Vec<i64> = codex_account::list_accounts()
        .into_iter()
        .filter(|account| selected.contains(account.id.as_str()))
        .flat_map(|account| account.quota.into_iter())
        .flat_map(|quota| {
            let mut values = Vec::new();
            if include_primary {
                values.push(quota.hourly_reset_time);
            }
            if include_secondary {
                values.push(quota.weekly_reset_time);
            }
            values
        })
        .flatten()
        .filter(|ts| *ts > 0)
        .collect();
    timestamps.sort_unstable();
    timestamps.dedup();
    timestamps
}

fn current_due_at(task: &codex_wakeup::CodexWakeupTask, now: DateTime<Local>) -> Option<i64> {
    match task.schedule.kind.as_str() {
        "daily" => {
            let minutes = parse_time_to_minutes(task.schedule.daily_time.as_deref()?)?;
            let candidate = build_local_datetime(now.date_naive(), minutes)?.timestamp();
            if candidate <= now.timestamp() && task.last_run_at.unwrap_or(0) < candidate {
                Some(candidate)
            } else {
                None
            }
        }
        "weekly" => {
            let minutes = parse_time_to_minutes(task.schedule.weekly_time.as_deref()?)?;
            let weekday = now.weekday().num_days_from_sunday() as i32;
            if !task.schedule.weekly_days.contains(&weekday) {
                return None;
            }
            let candidate = build_local_datetime(now.date_naive(), minutes)?.timestamp();
            if candidate <= now.timestamp() && task.last_run_at.unwrap_or(0) < candidate {
                Some(candidate)
            } else {
                None
            }
        }
        "interval" => {
            let interval_seconds =
                i64::from(task.schedule.interval_hours.unwrap_or(4).max(1)) * 3600;
            let due_at = task.last_run_at.unwrap_or(task.created_at) + interval_seconds;
            if due_at <= now.timestamp() {
                Some(due_at)
            } else {
                None
            }
        }
        "quota_reset" => {
            let last_run_at = task.last_run_at.unwrap_or(task.created_at);
            collect_task_reset_timestamps(task)
                .into_iter()
                .filter(|reset_at| *reset_at <= now.timestamp() && *reset_at > last_run_at)
                .max()
        }
        "startup" => None,
        _ => None,
    }
}

pub fn calculate_next_run_at(task: &codex_wakeup::CodexWakeupTask) -> Option<i64> {
    let now = Local::now();
    match task.schedule.kind.as_str() {
        "daily" => {
            let minutes = parse_time_to_minutes(task.schedule.daily_time.as_deref()?)?;
            for offset in 0..7 {
                let date = now.date_naive() + chrono::Duration::days(offset);
                let candidate = build_local_datetime(date, minutes)?.timestamp();
                if candidate > now.timestamp() {
                    return Some(candidate);
                }
            }
            None
        }
        "weekly" => {
            let minutes = parse_time_to_minutes(task.schedule.weekly_time.as_deref()?)?;
            for offset in 0..14 {
                let date = now.date_naive() + chrono::Duration::days(offset);
                let weekday = date.weekday().num_days_from_sunday() as i32;
                if !task.schedule.weekly_days.contains(&weekday) {
                    continue;
                }
                let candidate = build_local_datetime(date, minutes)?.timestamp();
                if candidate > now.timestamp() {
                    return Some(candidate);
                }
            }
            None
        }
        "interval" => {
            let interval_seconds =
                i64::from(task.schedule.interval_hours.unwrap_or(4).max(1)) * 3600;
            Some(task.last_run_at.unwrap_or(task.created_at) + interval_seconds)
        }
        "quota_reset" => collect_task_reset_timestamps(task)
            .into_iter()
            .filter(|reset_at| *reset_at > now.timestamp())
            .min(),
        "startup" => None,
        _ => None,
    }
}

fn mark_running(task_id: &str) -> bool {
    let mut guard = lock_or_recover(running_tasks(), "codex wakeup running tasks lock");
    guard.insert(task_id.to_string())
}

fn unmark_running(task_id: &str) {
    let mut guard = lock_or_recover(running_tasks(), "codex wakeup running tasks lock");
    guard.remove(task_id);
}

pub async fn run_task_now(
    app: Option<&AppHandle>,
    task_id: &str,
    trigger_type: &str,
    run_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    let task =
        codex_wakeup::get_task(task_id)?.ok_or_else(|| format!("唤醒任务不存在: {}", task_id))?;
    if !mark_running(&task.id) {
        return Err("该任务正在执行中".to_string());
    }

    let context = codex_wakeup::TaskRunContext {
        trigger_type: trigger_type.to_string(),
        task_id: Some(task.id.clone()),
        task_name: Some(task.name.clone()),
    };
    let result = codex_wakeup::run_batch(
        app,
        task.account_ids.clone(),
        task.prompt.clone(),
        codex_wakeup::CodexWakeupExecutionConfig {
            model: task.model.clone(),
            model_display_name: task.model_display_name.clone(),
            model_reasoning_effort: task.model_reasoning_effort.clone(),
        },
        context,
        run_id,
        None,
    )
    .await;

    if let Ok(batch) = &result {
        if let Err(err) = codex_wakeup::update_task_after_run(&task.id, &batch.records) {
            logger::log_warn(&format!("[CodexWakeup] 更新任务执行结果失败: {}", err));
        }
    }

    unmark_running(&task.id);
    result
}

pub async fn run_enabled_tasks_now(
    app: Option<&AppHandle>,
    trigger_type: &str,
) -> Result<u32, String> {
    let state = codex_wakeup::load_state_for_scheduler()?;
    if !state.enabled {
        return Ok(0);
    }

    let normalized_trigger = {
        let trimmed = trigger_type.trim();
        if trimmed.is_empty() {
            "startup"
        } else {
            trimmed
        }
    };

    if normalized_trigger == "startup" {
        let app_handle = app.cloned();
        let startup_tasks: Vec<(String, i32)> = state
            .tasks
            .into_iter()
            .filter(|task| task.enabled && task.schedule.kind == "startup")
            .map(|task| {
                (
                    task.id,
                    task.schedule.startup_delay_minutes.unwrap_or(0).max(0),
                )
            })
            .collect();

        for (task_id, delay_minutes) in &startup_tasks {
            let task_id = task_id.clone();
            let app_handle = app_handle.clone();
            let delay_seconds = (*delay_minutes as u64) * 60;
            tauri::async_runtime::spawn(async move {
                if delay_seconds > 0 {
                    sleep(Duration::from_secs(delay_seconds)).await;
                }

                let current_state = match codex_wakeup::load_state_for_scheduler() {
                    Ok(state) => state,
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[CodexWakeup] 读取启动后任务状态失败: task_id={}, error={}",
                            task_id, err
                        ));
                        return;
                    }
                };
                let should_run = current_state.enabled
                    && current_state.tasks.iter().any(|task| {
                        task.id == task_id && task.enabled && task.schedule.kind == "startup"
                    });
                if !should_run {
                    return;
                }

                if let Err(err) = run_task_now(app_handle.as_ref(), &task_id, "startup", None).await
                {
                    logger::log_warn(&format!(
                        "[CodexWakeup] 启动后执行任务失败: task_id={}, error={}",
                        task_id, err
                    ));
                }
            });
        }
        return Ok(startup_tasks.len() as u32);
    }

    let mut started_count: u32 = 0;
    for task in state.tasks {
        if !task.enabled || task.schedule.kind == "startup" {
            continue;
        }

        match run_task_now(app, &task.id, normalized_trigger, None).await {
            Ok(_) => {
                started_count += 1;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[CodexWakeup] 执行任务失败: task_id={}, error={}",
                    task.id, err
                ));
            }
        }
    }

    Ok(started_count)
}

pub fn trigger_startup_tasks_if_needed(app: AppHandle) {
    let state = match codex_wakeup::load_state_for_scheduler() {
        Ok(state) => state,
        Err(err) => {
            logger::log_warn(&format!("[CodexWakeup] 读取启动任务状态失败: {}", err));
            return;
        }
    };
    let has_startup_tasks = state
        .tasks
        .iter()
        .any(|task| task.enabled && task.schedule.kind == "startup");
    if !state.enabled || !has_startup_tasks {
        return;
    }

    let should_trigger = {
        let mut startup_triggered = lock_or_recover(
            startup_triggered_flag(),
            "codex wakeup startup trigger lock",
        );
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
        match run_enabled_tasks_now(Some(&app), "startup").await {
            Ok(started) => {
                if started > 0 {
                    logger::log_info(&format!(
                        "[CodexWakeup] 应用启动触发自启任务: started={}",
                        started
                    ));
                }
            }
            Err(err) => {
                logger::log_warn(&format!("[CodexWakeup] 应用启动触发自启任务失败: {}", err));
            }
        }
    });
}

async fn run_scheduler_once(app: &AppHandle) {
    let state = match codex_wakeup::load_state_for_scheduler() {
        Ok(state) => state,
        Err(err) => {
            logger::log_warn(&format!("[CodexWakeup] 读取任务状态失败: {}", err));
            return;
        }
    };

    if !state.enabled {
        return;
    }

    let now = Local::now();
    for task in state.tasks {
        if !task.enabled {
            continue;
        }
        if current_due_at(&task, now).is_none() {
            continue;
        }

        let task_id = task.id.clone();
        let trigger_type = if task.schedule.kind == "quota_reset" {
            "quota_reset"
        } else {
            "scheduled"
        }
        .to_string();
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let result = run_task_now(Some(&app_handle), &task_id, &trigger_type, None).await;
            if let Err(err) = result {
                logger::log_warn(&format!(
                    "[CodexWakeup] 调度任务执行失败: task_id={}, error={}",
                    task_id, err
                ));
            }
        });
    }
}

pub fn ensure_started(app: AppHandle) {
    let mut started = lock_or_recover(started_flag(), "codex wakeup scheduler started lock");
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
