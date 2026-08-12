//! Background tick for Kimi wakeup tasks (aligned with Codex scheduler semantics).

use crate::modules::{kimi_account, kimi_wakeup, logger};
use chrono::{DateTime, Datelike, Local, TimeZone};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;
use tokio::time::sleep;

static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static STARTUP_DONE: OnceLock<Mutex<bool>> = OnceLock::new();

fn started_flag() -> &'static Mutex<bool> {
    STARTED.get_or_init(|| Mutex::new(false))
}

fn running_tasks() -> &'static Mutex<HashSet<String>> {
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

fn startup_flag() -> &'static Mutex<bool> {
    STARTUP_DONE.get_or_init(|| Mutex::new(false))
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
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

fn collect_quota_reset_ts(task: &kimi_wakeup::KimiWakeupTask) -> Vec<i64> {
    let window = task
        .schedule
        .quota_reset_window
        .as_deref()
        .unwrap_or("either");
    let include_primary = window == "either" || window == "primary_window";
    let include_secondary = window == "either" || window == "secondary_window";
    let selected: HashSet<&str> = task.account_ids.iter().map(String::as_str).collect();
    let mut ts = Vec::new();
    if let Ok(views) = kimi_account::list_accounts_checked() {
        for view in views {
            if !selected.contains(view.id.as_str()) {
                continue;
            }
            if let Some(account) = kimi_account::load_account(&view.id) {
                if let Some(quota) = account.quota {
                    if include_primary {
                        if let Some(reset) = quota.weekly_reset_at.as_deref() {
                            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset) {
                                ts.push(dt.timestamp());
                            } else if let Ok(n) = reset.parse::<i64>() {
                                ts.push(if n > 1_000_000_000_000 { n / 1000 } else { n });
                            }
                        }
                    }
                    if include_secondary {
                        for row in quota.limits {
                            if let Some(reset) = row.reset_at.as_deref() {
                                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset) {
                                    ts.push(dt.timestamp());
                                } else if let Ok(n) = reset.parse::<i64>() {
                                    ts.push(if n > 1_000_000_000_000 { n / 1000 } else { n });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    ts.sort_unstable();
    ts.dedup();
    ts
}

/// Returns the due timestamp when the task should run now, or `None` if not due.
/// Public for unit tests and UI next-run helpers.
pub fn current_due_at(task: &kimi_wakeup::KimiWakeupTask, now: DateTime<Local>) -> Option<i64> {
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
            // Align with Codex: first run is created_at + interval, never "immediate on next tick".
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
            let last = task.last_run_at.unwrap_or(task.created_at);
            collect_quota_reset_ts(task)
                .into_iter()
                .filter(|ts| *ts <= now.timestamp() && *ts > last)
                .max()
        }
        "startup" => None, // handled once via trigger_startup_tasks_if_needed
        _ => None,
    }
}

fn mark_running(task_id: &str) -> bool {
    lock_or_recover(running_tasks()).insert(task_id.to_string())
}

fn unmark_running(task_id: &str) {
    lock_or_recover(running_tasks()).remove(task_id);
}

/// Run a single task off the async runtime (blocking CLI work).
async fn run_task_now(task_id: &str, trigger_type: &str) -> Result<(), String> {
    if !mark_running(task_id) {
        return Err("该任务正在执行中".to_string());
    }
    let id = task_id.to_string();
    let trigger = trigger_type.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || kimi_wakeup::run_task(&id, &trigger))
        .await;
    unmark_running(task_id);
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(format!("join 失败: {}", e)),
    }
}

pub fn ensure_started(app: AppHandle) {
    let mut started = lock_or_recover(started_flag());
    if *started {
        return;
    }
    *started = true;
    drop(started);
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(Duration::from_secs(30)).await;
            if let Err(e) = tick_once().await {
                logger::log_warn(&format!("[KimiWakeupScheduler] tick 失败: {}", e));
            }
            let _ = &app;
        }
    });
}

pub fn trigger_startup_tasks_if_needed(_app: AppHandle) {
    let mut done = lock_or_recover(startup_flag());
    if *done {
        return;
    }
    *done = true;
    drop(done);

    tauri::async_runtime::spawn(async move {
        // Brief settle delay so app + account store are ready (shared base, not per-task).
        sleep(Duration::from_secs(15)).await;

        let state = match kimi_wakeup::load_state() {
            Ok(s) => s,
            Err(e) => {
                logger::log_warn(&format!(
                    "[KimiWakeupScheduler] 读取启动任务状态失败: {}",
                    e
                ));
                return;
            }
        };
        if !state.enabled {
            return;
        }

        let startup_tasks: Vec<(String, u64)> = state
            .tasks
            .into_iter()
            .filter(|t| t.enabled && t.schedule.kind == "startup")
            .map(|t| {
                (
                    t.id,
                    t.schedule.startup_delay_minutes.unwrap_or(0).max(0) as u64 * 60,
                )
            })
            .collect();

        // Each startup task gets its own timer — delays do not stack.
        for (task_id, delay_seconds) in startup_tasks {
            tauri::async_runtime::spawn(async move {
                if delay_seconds > 0 {
                    sleep(Duration::from_secs(delay_seconds)).await;
                }

                let current = match kimi_wakeup::load_state() {
                    Ok(s) => s,
                    Err(e) => {
                        logger::log_warn(&format!(
                            "[KimiWakeupScheduler] 启动任务重读状态失败: {} {}",
                            task_id, e
                        ));
                        return;
                    }
                };
                let should_run = current.enabled
                    && current.tasks.iter().any(|t| {
                        t.id == task_id && t.enabled && t.schedule.kind == "startup"
                    });
                if !should_run {
                    return;
                }

                if let Err(e) = run_task_now(&task_id, "startup").await {
                    logger::log_warn(&format!(
                        "[KimiWakeupScheduler] startup 任务失败: {} {}",
                        task_id, e
                    ));
                }
            });
        }
    });
}

async fn tick_once() -> Result<(), String> {
    let state = kimi_wakeup::load_state()?;
    if !state.enabled {
        return Ok(());
    }
    let now = Local::now();
    for task in state.tasks.into_iter().filter(|t| t.enabled) {
        if task.schedule.kind == "startup" {
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
        // Fire-and-forget: a long CLI run must not block evaluating other due tasks.
        tauri::async_runtime::spawn(async move {
            if let Err(e) = run_task_now(&task_id, &trigger_type).await {
                logger::log_warn(&format!(
                    "[KimiWakeupScheduler] 任务失败 {}: {}",
                    task_id, e
                ));
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::kimi_wakeup::{KimiWakeupSchedule, KimiWakeupTask};
    use chrono::TimeZone;

    fn sample_task(kind: &str) -> KimiWakeupTask {
        KimiWakeupTask {
            id: "t1".into(),
            name: "test".into(),
            enabled: true,
            account_ids: vec!["a1".into()],
            prompt: Some("hi".into()),
            model: None,
            schedule: KimiWakeupSchedule {
                kind: kind.into(),
                daily_time: Some("08:00".into()),
                weekly_days: vec![0, 1, 2, 3, 4, 5, 6],
                weekly_time: Some("08:00".into()),
                interval_hours: Some(6),
                quota_reset_window: None,
                startup_delay_minutes: None,
            },
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            last_run_at: None,
            last_status: None,
            last_message: None,
            last_success_count: None,
            last_failure_count: None,
            last_duration_ms: None,
        }
    }

    #[test]
    fn interval_not_due_immediately_when_last_run_missing() {
        let task = sample_task("interval");
        // now = created_at + 1h → still inside first 6h window
        let now = Local
            .timestamp_opt(task.created_at + 3600, 0)
            .single()
            .expect("ts");
        assert!(current_due_at(&task, now).is_none());
    }

    #[test]
    fn interval_due_after_created_plus_interval() {
        let task = sample_task("interval");
        let now = Local
            .timestamp_opt(task.created_at + 6 * 3600 + 1, 0)
            .single()
            .expect("ts");
        let due = current_due_at(&task, now).expect("due");
        assert_eq!(due, task.created_at + 6 * 3600);
    }

    #[test]
    fn interval_uses_last_run_when_present() {
        let mut task = sample_task("interval");
        task.last_run_at = Some(task.created_at + 100);
        let now = Local
            .timestamp_opt(task.created_at + 100 + 6 * 3600 + 5, 0)
            .single()
            .expect("ts");
        let due = current_due_at(&task, now).expect("due");
        assert_eq!(due, task.created_at + 100 + 6 * 3600);
    }

    #[test]
    fn interval_hours_floor_at_one() {
        let mut task = sample_task("interval");
        task.schedule.interval_hours = Some(0);
        let now = Local
            .timestamp_opt(task.created_at + 3600 + 1, 0)
            .single()
            .expect("ts");
        // max(1) hour → due at created+1h
        assert!(current_due_at(&task, now).is_some());
    }

    #[test]
    fn daily_due_once_per_slot() {
        let mut task = sample_task("daily");
        task.schedule.daily_time = Some("08:00".into());
        // Pick a local morning after 08:00
        let now = Local.with_ymd_and_hms(2024, 6, 1, 9, 0, 0).single().unwrap();
        assert!(current_due_at(&task, now).is_some());
        task.last_run_at = Some(now.timestamp());
        assert!(current_due_at(&task, now).is_none());
    }

    #[test]
    fn startup_kind_never_due_via_tick() {
        let task = sample_task("startup");
        let now = Local::now();
        assert!(current_due_at(&task, now).is_none());
    }
}
