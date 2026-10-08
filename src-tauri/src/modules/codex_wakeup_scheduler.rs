use crate::modules::{codex_account, codex_wakeup, logger};
use chrono::{DateTime, Datelike, Local, TimeZone};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;
use tokio::time::sleep;

static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
static RUNNING_TASKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static ACTIVE_RUN_SCOPES: OnceLock<Mutex<HashMap<String, ActiveRunScope>>> = OnceLock::new();
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

pub(super) fn current_due_at(
    task: &codex_wakeup::CodexWakeupTask,
    now: DateTime<Local>,
) -> Option<i64> {
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

struct ActiveRunScope {
    scope: String,
    automatic: bool,
}

fn active_run_scopes() -> &'static Mutex<HashMap<String, ActiveRunScope>> {
    ACTIVE_RUN_SCOPES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn cancel_disabled_tasks(state: &codex_wakeup::CodexWakeupState) {
    let scopes = lock_or_recover(active_run_scopes(), "codex wakeup scopes lock");
    for (task_id, active) in scopes.iter() {
        let task = state.tasks.iter().find(|task| task.id == *task_id);
        let should_cancel = task.is_none()
            || (active.automatic && (!state.enabled || task.is_some_and(|task| !task.enabled)));
        if should_cancel {
            if let Err(error) = codex_wakeup::cancel_wakeup_scope(&active.scope) {
                logger::log_warn(&format!("[CodexWakeup] 取消任务失败: {}", error));
            }
        }
    }
}

struct TaskRunGuard {
    task_id: String,
    scope: String,
    _lease: std::fs::File,
}

impl Drop for TaskRunGuard {
    fn drop(&mut self) {
        lock_or_recover(active_run_scopes(), "codex wakeup scopes lock").remove(&self.task_id);
        let _ = codex_wakeup::release_wakeup_scope(&self.scope);
        unmark_running(&self.task_id);
    }
}

pub async fn run_task_now(
    app: Option<&AppHandle>,
    task_id: &str,
    trigger_type: &str,
    run_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    run_task(app, task_id, trigger_type, run_id, None)
        .await?
        .ok_or_else(|| "唤醒任务已停用、删除或正在执行中".to_string())
}

async fn run_task(
    app: Option<&AppHandle>,
    task_id: &str,
    trigger_type: &str,
    run_id: Option<String>,
    due_at: Option<i64>,
) -> Result<Option<codex_wakeup::CodexWakeupBatchResult>, String> {
    let Some(lease) = codex_wakeup::try_task_run_lease(task_id)? else {
        return Ok(None);
    };
    if !mark_running(task_id) {
        return Ok(None);
    }
    let scope = format!("codex-wakeup-task:{}:{}", task_id, uuid::Uuid::new_v4());
    let _guard = TaskRunGuard {
        task_id: task_id.to_string(),
        scope: scope.clone(),
        _lease: lease,
    };
    codex_wakeup::resolve_cancel_flag(Some(&scope))?;
    let require_enabled = trigger_type != "manual_task";
    lock_or_recover(active_run_scopes(), "codex wakeup scopes lock").insert(
        task_id.to_string(),
        ActiveRunScope {
            scope: scope.clone(),
            automatic: require_enabled,
        },
    );

    // Register cancellation first, then re-read enabled state while claiming.
    // A disable racing either side of the claim therefore cannot be lost.
    let Some(task) = codex_wakeup::claim_task_run(task_id, due_at, require_enabled)? else {
        return Ok(None);
    };
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
        Some(&scope),
    )
    .await;

    let update_result = match &result {
        Ok(batch) => codex_wakeup::update_task_after_run(&task.id, &batch.records),
        Err(error) => codex_wakeup::mark_task_run_failed(&task.id, error),
    };
    if let Err(error) = update_result {
        logger::log_warn(&format!("[CodexWakeup] 更新任务执行结果失败: {}", error));
    }
    // Refresh reset windows through the existing deduplicating background
    // queue, after committing the run result. No scheduler lock spans I/O.
    if task.schedule.kind == "quota_reset" && result.is_ok() {
        let ids = task.account_ids.clone();
        tauri::async_runtime::spawn(async move {
            match tokio::time::timeout(
                Duration::from_secs(60),
                crate::modules::codex_quota::refresh_quotas_for_account_ids_in_background(
                    &ids, true,
                ),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    logger::log_warn(&format!("[CodexWakeup] 更新重置时间失败: {}", error))
                }
                Err(_) => logger::log_warn("[CodexWakeup] 更新重置时间超时"),
            }
        });
    }
    result.map(Some)
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

    cancel_disabled_tasks(&state);
    if !state.enabled {
        return;
    }

    let now = Local::now();
    for task in state.tasks {
        if !task.enabled {
            continue;
        }
        let Some(due_at) = current_due_at(&task, now) else {
            continue;
        };
        if lock_or_recover(running_tasks(), "codex wakeup running tasks lock").contains(&task.id) {
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
            let result = run_task(
                Some(&app_handle),
                &task_id,
                &trigger_type,
                None,
                Some(due_at),
            )
            .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn disabling_automation_cancels_scheduled_runs_but_preserves_explicit_manual_runs() {
        let automatic_id = uuid::Uuid::new_v4().to_string();
        let manual_id = uuid::Uuid::new_v4().to_string();
        let deleted_id = uuid::Uuid::new_v4().to_string();
        let flags: Vec<_> = [&automatic_id, &manual_id, &deleted_id]
            .iter()
            .map(|id| {
                codex_wakeup::resolve_cancel_flag(Some(id))
                    .unwrap()
                    .unwrap()
            })
            .collect();
        let tasks: Vec<_> = [&automatic_id, &manual_id]
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id, "name": "Task", "enabled": false, "accountIds": ["account"],
                    "createdAt": 1, "updatedAt": 1,
                    "schedule": {"kind": "interval", "weeklyDays": [], "intervalHours": 1}
                })
            })
            .collect();
        let state: codex_wakeup::CodexWakeupState = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "tasks": tasks
        }))
        .unwrap();
        {
            let mut scopes = active_run_scopes().lock().unwrap();
            scopes.insert(
                automatic_id.clone(),
                ActiveRunScope {
                    scope: automatic_id.clone(),
                    automatic: true,
                },
            );
            scopes.insert(
                manual_id.clone(),
                ActiveRunScope {
                    scope: manual_id.clone(),
                    automatic: false,
                },
            );
            scopes.insert(
                deleted_id.clone(),
                ActiveRunScope {
                    scope: deleted_id.clone(),
                    automatic: false,
                },
            );
        }
        cancel_disabled_tasks(&state);
        assert!(flags[0].load(Ordering::SeqCst));
        assert!(!flags[1].load(Ordering::SeqCst));
        assert!(flags[2].load(Ordering::SeqCst));
        for id in [&automatic_id, &manual_id, &deleted_id] {
            active_run_scopes().lock().unwrap().remove(id);
            codex_wakeup::release_wakeup_scope(id).unwrap();
        }
    }
}
