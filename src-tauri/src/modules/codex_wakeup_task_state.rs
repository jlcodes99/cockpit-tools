//! Durable scheduler claims and cancellation. Configuration saves and task
//! completion both merge against the current state under the same short lock.
use super::*;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::future::Future;

fn try_file_lock(path: &Path) -> Result<Option<fs::File>, String> {
    let parent = path.parent().ok_or("无法定位 Codex 唤醒锁目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建 Codex 唤醒锁目录失败: {}", e))?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("打开 Codex 唤醒锁失败: {}", e))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(file)),
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(None)
        }
        Err(error) => Err(format!("获取 Codex 唤醒锁失败: {}", error)),
    }
}

pub(super) fn lock_task_state() -> Result<fs::File, String> {
    try_file_lock(&tasks_path()?.with_extension("json.lock"))?
        .ok_or_else(|| "Codex 唤醒任务状态正在更新，请重试".to_string())
}

pub fn try_task_run_lease(task_id: &str) -> Result<Option<fs::File>, String> {
    let filename = format!("{:x}.lock", Sha256::digest(task_id.as_bytes()));
    try_file_lock(&data_dir()?.join("codex_wakeup_run_locks").join(filename))
}

pub(super) fn preserve_run_metadata(next: &mut CodexWakeupState, current: &CodexWakeupState) {
    for task in &mut next.tasks {
        if let Some(saved) = current.tasks.iter().find(|saved| saved.id == task.id) {
            task.last_run_at = saved.last_run_at;
            task.last_status = saved.last_status.clone();
            task.last_message = saved.last_message.clone();
            task.last_success_count = saved.last_success_count;
            task.last_failure_count = saved.last_failure_count;
            task.last_duration_ms = saved.last_duration_ms;
        }
    }
}

fn claim_in_state(
    state: &mut CodexWakeupState,
    task_id: &str,
    due_at: Option<i64>,
    require_enabled: bool,
    now: i64,
) -> Option<CodexWakeupTask> {
    if require_enabled && !state.enabled {
        return None;
    }
    let task = state.tasks.iter_mut().find(|task| task.id == task_id)?;
    if require_enabled && !task.enabled {
        return None;
    }
    if due_at.is_some_and(|due| due > now || task.last_run_at.unwrap_or(0) >= due) {
        return None;
    }
    task.last_run_at = Some(now);
    task.last_status = Some("running".to_string());
    task.last_message = None;
    task.last_success_count = None;
    task.last_failure_count = None;
    task.last_duration_ms = None;
    task.updated_at = now;
    Some(task.clone())
}

/// Called only while holding this task's run lease. A durable claim is written
/// before dispatch, so failures and restarts cannot blindly replay a paid call.
pub fn claim_task_run(
    task_id: &str,
    due_at: Option<i64>,
    require_enabled: bool,
) -> Result<Option<CodexWakeupTask>, String> {
    let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
    let _file_lock = lock_task_state()?;
    let mut state = read_state_file()?;
    if let Some(expected_due) = due_at {
        let still_due = state
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .and_then(|task| {
                crate::modules::codex_wakeup_scheduler::current_due_at(task, chrono::Local::now())
            });
        if still_due != Some(expected_due) {
            return Ok(None);
        }
    }
    let Some(task) = claim_in_state(&mut state, task_id, due_at, require_enabled, now_ts()) else {
        return Ok(None);
    };
    refresh_next_run_at(&mut state);
    save_json_atomic(&tasks_path()?, &state)?;
    Ok(Some(task))
}

fn record_dispatch_failure(task: &mut CodexWakeupTask, error: Option<&str>) {
    task.last_status = Some("error".to_string());
    task.last_message = error.map(str::to_string);
    task.last_success_count = Some(0);
    task.last_failure_count = Some(1);
    task.last_duration_ms = None;
    task.updated_at = now_ts();
    // Keep last_run_at: failure never re-arms the same due event.
}

pub fn mark_task_run_failed(task_id: &str, error: &str) -> Result<(), String> {
    let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
    let _file_lock = lock_task_state()?;
    let mut state = read_state_file()?;
    if let Some(task) = state.tasks.iter_mut().find(|task| task.id == task_id) {
        record_dispatch_failure(task, Some(error));
        refresh_next_run_at(&mut state);
        save_json_atomic(&tasks_path()?, &state)?;
    }
    Ok(())
}

pub(super) fn recover_interrupted_runs(state: &mut CodexWakeupState) -> Result<bool, String> {
    recover_interrupted_runs_with(state, try_task_run_lease)
}

fn recover_interrupted_runs_with(
    state: &mut CodexWakeupState,
    mut acquire: impl FnMut(&str) -> Result<Option<fs::File>, String>,
) -> Result<bool, String> {
    let mut changed = false;
    for task in &mut state.tasks {
        if task.last_status.as_deref() != Some("running") {
            continue;
        }
        let Some(_lease) = acquire(&task.id)? else {
            continue;
        };
        // The process holding the run lease exited. The upstream may already
        // have accepted the request; expose failure and leave manual retry
        // available instead of automatically spending quota again.
        record_dispatch_failure(task, None);
        logger::log_warn(&format!(
            "[CodexWakeup] 恢复中断任务，保留已认领事件并等待下次计划或手动重试: task_id={}",
            task.id
        ));
        changed = true;
    }
    Ok(changed)
}

pub(super) async fn await_with_cancel<T, F>(
    flag: Option<&Arc<AtomicBool>>,
    future: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    let timed_request = async {
        tokio::time::timeout(Duration::from_secs(180), future)
            .await
            .map_err(|_| "Codex 唤醒请求超时".to_string())?
    };
    let Some(flag) = flag else {
        return timed_request.await;
    };
    tokio::select! {
        biased;
        _ = async {
            loop {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(CODEX_WAKEUP_CANCEL_POLL_MS)).await;
            }
        } => Err(cancelled_error()),
        result = timed_request => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> CodexWakeupState {
        serde_json::from_value(serde_json::json!({
            "enabled": true,
            "tasks": [{
                "id": "task-1", "name": "Task", "enabled": true,
                "accountIds": ["account-1"], "createdAt": 1, "updatedAt": 1,
                "schedule": { "kind": "interval", "weeklyDays": [], "intervalHours": 1 }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn durable_claim_prevents_replay_after_serialization_and_failure() {
        let mut state = state();
        assert!(claim_in_state(&mut state, "task-1", Some(100), true, 120).is_some());
        let encoded = serde_json::to_string(&state).unwrap();
        let mut restored: CodexWakeupState = serde_json::from_str(&encoded).unwrap();
        record_dispatch_failure(&mut restored.tasks[0], Some("network failed"));
        assert!(claim_in_state(&mut restored, "task-1", Some(100), true, 180).is_none());
        assert_eq!(restored.tasks[0].last_run_at, Some(120));
        assert!(claim_in_state(&mut restored, "task-1", Some(200), true, 220).is_some());
    }

    #[test]
    fn claims_recheck_disabled_state_but_allow_explicit_manual_retry() {
        let mut state = state();
        state.enabled = false;
        assert!(claim_in_state(&mut state, "task-1", None, true, 100).is_none());
        assert!(claim_in_state(&mut state, "task-1", None, false, 100).is_some());
        state.enabled = true;
        state.tasks[0].enabled = false;
        assert!(claim_in_state(&mut state, "task-1", Some(200), true, 220).is_none());
    }

    #[test]
    fn stale_ui_save_cannot_undo_claim_or_discard_failure() {
        let mut saved = state();
        let mut stale_ui = saved.clone();
        claim_in_state(&mut saved, "task-1", Some(100), true, 120).unwrap();
        record_dispatch_failure(&mut saved.tasks[0], Some("cancelled"));
        stale_ui.enabled = false;
        stale_ui.tasks[0].name = "Edited name".to_string();
        preserve_run_metadata(&mut stale_ui, &saved);
        assert!(!stale_ui.enabled);
        assert_eq!(stale_ui.tasks[0].name, "Edited name");
        assert_eq!(stale_ui.tasks[0].last_run_at, Some(120));
        assert_eq!(stale_ui.tasks[0].last_status.as_deref(), Some("error"));
    }

    #[test]
    fn cancellation_before_scope_resolution_is_retained_until_release() {
        let id = uuid::Uuid::new_v4().to_string();
        cancel_wakeup_scope(&id).unwrap();
        let flag = resolve_cancel_flag(Some(&id)).unwrap().unwrap();
        assert!(is_scope_cancelled(Some(&flag)));
        release_wakeup_scope(&id).unwrap();
        let fresh = resolve_cancel_flag(Some(&id)).unwrap().unwrap();
        assert!(!is_scope_cancelled(Some(&fresh)));
        release_wakeup_scope(&id).unwrap();
    }

    #[test]
    fn task_file_lock_is_nonblocking_and_recovers_after_owner_drops() {
        let dir = std::env::temp_dir().join(format!("wakeup-lock-{}", uuid::Uuid::new_v4()));
        let path = dir.join("task.lock");
        let lease = try_file_lock(&path).unwrap().unwrap();
        assert!(try_file_lock(&path).unwrap().is_none());
        drop(lease);
        drop(try_file_lock(&path).unwrap().unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interrupted_run_recovers_without_replaying_and_leaves_active_run_intact() {
        let mut state = state();
        claim_in_state(&mut state, "task-1", Some(100), true, 120).unwrap();
        assert!(!recover_interrupted_runs_with(&mut state, |_| Ok(None)).unwrap());
        assert_eq!(state.tasks[0].last_status.as_deref(), Some("running"));
        let dir = std::env::temp_dir().join(format!("wakeup-recover-{}", uuid::Uuid::new_v4()));
        let path = dir.join("task.lock");
        assert!(recover_interrupted_runs_with(&mut state, |_| try_file_lock(&path)).unwrap());
        assert_eq!(state.tasks[0].last_status.as_deref(), Some("error"));
        assert_eq!(state.tasks[0].last_run_at, Some(120));
        assert!(claim_in_state(&mut state, "task-1", Some(100), true, 180).is_none());
        assert!(claim_in_state(&mut state, "task-1", None, false, 180).is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn cancelled_scope_never_polls_the_request() {
        let flag = Arc::new(AtomicBool::new(true));
        let result = await_with_cancel(Some(&flag), async {
            panic!("cancelled request must not start");
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        })
        .await;
        assert_eq!(result, Err(cancelled_error()));
    }

    #[tokio::test]
    async fn cancellation_drops_an_inflight_request_future() {
        struct DropSignal(Arc<AtomicBool>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let pending_signal = DropSignal(dropped.clone());
        let request = async move {
            let _signal = pending_signal;
            std::future::pending::<Result<(), String>>().await
        };
        let cancel_for_timer = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_for_timer.store(true, Ordering::SeqCst);
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            await_with_cancel(Some(&cancel), request),
        )
        .await
        .unwrap();
        assert_eq!(result, Err(cancelled_error()));
        assert!(dropped.load(Ordering::SeqCst));
    }
}
