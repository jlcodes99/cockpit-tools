use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ::codex_task_supervisor::{
    classify_codex_event, CodexEventSource, CodexEvidenceKind, CodexFailureClass,
    CodexSupervisorAction, CodexTaskEvidence, EvidenceCursor, ManagedCodexAccountScope,
    ManagedCodexTask, ManagedCodexTaskConfig, ManagedCodexTaskStatus,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::{Mutex as TokioMutex, Notify};

use crate::modules::{
    app_lifecycle, codex_account, codex_local_access, codex_task_store, codex_wakeup, logger,
    process,
};

pub const UPDATED_EVENT: &str = "codex-managed-task://updated";
pub const EVIDENCE_EVENT: &str = "codex-managed-task://evidence";

const MAX_TASK_LIST_LIMIT: usize = 1_000;
const MAX_PERSISTED_ERROR_CHARS: usize = 2_000;
const MAX_STDERR_SUMMARY_CHARS: usize = 16_000;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(40);

static RUNTIME_STARTED: AtomicBool = AtomicBool::new(false);
static RUNTIME_STATE: OnceLock<Arc<TokioMutex<ManagedTaskRuntimeState>>> = OnceLock::new();
static RUNTIME_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

#[derive(Default)]
struct ManagedTaskRuntimeState {
    active_task_id: Option<String>,
    cancel_flags: HashMap<String, Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexTaskRuntimeStatus {
    pub cli: codex_wakeup::CodexCliStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_task_id: Option<String>,
    pub queue_length: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexTaskEvidencePage {
    pub items: Vec<CodexTaskEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<EvidenceCursor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedCodexEvidenceEventPayload {
    task_id: String,
    evidence: CodexTaskEvidence,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCodexTaskResumeMode {
    SameAccount,
    NextEligible,
}

#[derive(Debug, Clone)]
enum ProcessLaunchMode {
    Initial { objective: String },
    Resume { thread_id: String, prompt: String },
}

struct ProcessOutcome {
    status: Option<ExitStatus>,
    cancelled: bool,
    saw_terminal: bool,
    last_structured_error: Option<CodexTaskEvidence>,
    stderr_summary: String,
    malformed_line_count: u64,
}

enum RecordedProcessState {
    NotRunning,
    MatchingAlive,
    PidReused,
}

enum AppServerRecoveryResult {
    Completed(CodexTaskEvidence),
    QuotaFailed(CodexTaskEvidence),
    NonQuotaFailed(CodexTaskEvidence),
    Interrupted(CodexTaskEvidence),
    Ambiguous(String),
}

fn runtime_state() -> &'static Arc<TokioMutex<ManagedTaskRuntimeState>> {
    RUNTIME_STATE.get_or_init(|| Arc::new(TokioMutex::new(ManagedTaskRuntimeState::default())))
}

fn runtime_notify() -> &'static Arc<Notify> {
    RUNTIME_NOTIFY.get_or_init(|| Arc::new(Notify::new()))
}

fn store() -> Result<codex_task_store::CodexTaskStore, String> {
    codex_task_store::default_codex_task_store()
}

pub fn ensure_started(app: AppHandle) {
    if RUNTIME_STARTED.swap(true, Ordering::SeqCst) {
        runtime_notify().notify_one();
        return;
    }

    tauri::async_runtime::spawn(async move {
        if let Err(error) = initialize_and_recover(&app).await {
            logger::log_error(&format!(
                "[ManagedCodexTask] initialize/recover failed: {}",
                sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
            ));
        }
        scheduler_loop(app).await;
    });
}

pub fn shutdown() {
    if let Some(state) = RUNTIME_STATE.get() {
        if let Ok(state) = state.try_lock() {
            if let Some(task_id) = state.active_task_id.as_ref() {
                if let Some(flag) = state.cancel_flags.get(task_id) {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
    }
    runtime_notify().notify_waiters();
}

pub async fn shutdown_and_wait(app: &AppHandle) {
    shutdown();
    let completed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if runtime_state().lock().await.active_task_id.is_none() {
                return;
            }
            tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
        }
    })
    .await
    .is_ok();
    if completed {
        return;
    }

    let active_task_id = runtime_state().lock().await.active_task_id.clone();
    let Some(task_id) = active_task_id else {
        return;
    };
    if let Ok(store) = store() {
        if let Ok(Some(mut task)) = store.get_task(&task_id) {
            if matches!(
                recorded_process_state(&task),
                RecordedProcessState::MatchingAlive
            ) {
                if let Some(pid) = task.process_id {
                    let _ = terminate_pid_tree(pid).await;
                }
            }
            cleanup_task_credentials(&task).await;
            task.clear_process();
            task.mark_cancelled("Cockpit exited while the managed Codex task was active");
            let _ = store.save_task(&task);
            emit_task_updated(app, &task);
        }
    }
    let mut state = runtime_state().lock().await;
    state.cancel_flags.remove(&task_id);
    if state.active_task_id.as_deref() == Some(task_id.as_str()) {
        state.active_task_id = None;
    }
}

pub async fn create_task(
    app: &AppHandle,
    config: ManagedCodexTaskConfig,
) -> Result<ManagedCodexTask, String> {
    let cli = codex_wakeup::get_cli_status();
    if !cli.available {
        return Err(cli.message.unwrap_or_else(|| {
            "Codex CLI is unavailable; configure an executable CLI first".to_string()
        }));
    }

    let config = validate_task_config(config)?;
    let task = ManagedCodexTask::create(config)?;
    let store = store()?;
    store.save_task(&task)?;
    emit_task_updated(app, &task);
    runtime_notify().notify_one();
    Ok(task)
}

pub async fn list_tasks(limit: Option<usize>) -> Result<Vec<ManagedCodexTask>, String> {
    let store = store()?;
    let mut tasks = store.list_tasks(limit.map(|value| value.clamp(1, MAX_TASK_LIST_LIMIT)))?;
    apply_queue_positions(&mut tasks);
    Ok(tasks)
}

pub async fn get_task(task_id: &str) -> Result<ManagedCodexTask, String> {
    let store = store()?;
    store
        .get_task(task_id.trim())?
        .ok_or_else(|| format!("managed Codex task does not exist: {}", task_id.trim()))
}

pub async fn list_evidence(
    task_id: &str,
    cursor: Option<EvidenceCursor>,
    limit: Option<usize>,
) -> Result<ManagedCodexTaskEvidencePage, String> {
    let store = store()?;
    if store.get_task(task_id.trim())?.is_none() {
        return Err(format!(
            "managed Codex task does not exist: {}",
            task_id.trim()
        ));
    }
    let items = store.list_evidence_page(task_id, cursor.as_ref(), limit)?;
    let next_cursor = items.first().map(|evidence| EvidenceCursor {
        observed_at: evidence.observed_at,
        id: evidence.id.clone(),
    });
    Ok(ManagedCodexTaskEvidencePage { items, next_cursor })
}

pub async fn runtime_status() -> Result<ManagedCodexTaskRuntimeStatus, String> {
    let active_task_id = runtime_state().lock().await.active_task_id.clone();
    let queue_length = store()?
        .list_tasks(Some(MAX_TASK_LIST_LIMIT))?
        .into_iter()
        .filter(|task| is_scheduler_runnable(task.status))
        .count();
    Ok(ManagedCodexTaskRuntimeStatus {
        cli: codex_wakeup::get_cli_status(),
        active_task_id,
        queue_length,
    })
}

pub async fn cancel_task(app: &AppHandle, task_id: &str) -> Result<ManagedCodexTask, String> {
    let task_id = task_id.trim();
    let store = store()?;
    let mut task = store
        .get_task(task_id)?
        .ok_or_else(|| format!("managed Codex task does not exist: {task_id}"))?;
    if task.status.is_final() {
        return Ok(task);
    }

    let active_flag = {
        let state = runtime_state().lock().await;
        state.cancel_flags.get(task_id).cloned()
    };
    if let Some(flag) = active_flag {
        flag.store(true, Ordering::SeqCst);
        runtime_notify().notify_one();
        return Ok(task);
    }

    if matches!(
        recorded_process_state(&task),
        RecordedProcessState::MatchingAlive
    ) {
        if let Some(pid) = task.process_id {
            terminate_pid_tree(pid).await?;
        }
    }
    cleanup_task_credentials(&task).await;
    task.clear_process();
    task.mark_cancelled("managed Codex task was cancelled by the user");
    store.save_task(&task)?;
    emit_task_updated(app, &task);
    runtime_notify().notify_one();
    Ok(task)
}

pub async fn resume_task(
    app: &AppHandle,
    task_id: &str,
    mode: ManagedCodexTaskResumeMode,
) -> Result<ManagedCodexTask, String> {
    let store = store()?;
    let mut task = store
        .get_task(task_id.trim())?
        .ok_or_else(|| format!("managed Codex task does not exist: {}", task_id.trim()))?;
    if matches!(
        recorded_process_state(&task),
        RecordedProcessState::MatchingAlive
    ) {
        if let Some(pid) = task.process_id {
            terminate_pid_tree(pid).await?;
        }
    }
    cleanup_task_credentials(&task).await;
    match mode {
        ManagedCodexTaskResumeMode::SameAccount => {
            task.prepare_manual_resume_same_account()?;
        }
        ManagedCodexTaskResumeMode::NextEligible => {
            if task.thread_id.is_none() {
                task.requeue_before_first_launch()?;
            } else {
                task.prepare_manual_resume_next_account()?;
            }
        }
    }
    task.clear_process();
    store.save_task(&task)?;
    emit_task_updated(app, &task);
    runtime_notify().notify_one();
    Ok(task)
}

fn validate_task_config(config: ManagedCodexTaskConfig) -> Result<ManagedCodexTaskConfig, String> {
    let config = config.normalized()?;
    let cwd = PathBuf::from(&config.cwd);
    if !cwd.is_absolute() {
        return Err("managed Codex task working directory must be absolute".to_string());
    }
    if !cwd.is_dir() {
        return Err(format!(
            "managed Codex task working directory does not exist: {}",
            cwd.display()
        ));
    }
    Ok(config)
}

fn apply_queue_positions(tasks: &mut [ManagedCodexTask]) {
    let mut queued = tasks
        .iter()
        .filter(|task| is_scheduler_runnable(task.status))
        .map(|task| (task.created_at, task.id.clone()))
        .collect::<Vec<_>>();
    queued.sort_by(|left, right| left.cmp(right));
    let positions = queued
        .into_iter()
        .enumerate()
        .map(|(index, (_, id))| (id, (index + 1) as u32))
        .collect::<HashMap<_, _>>();
    for task in tasks {
        task.queue_position = positions.get(&task.id).copied();
    }
}

fn is_scheduler_runnable(status: ManagedCodexTaskStatus) -> bool {
    matches!(
        status,
        ManagedCodexTaskStatus::Queued
            | ManagedCodexTaskStatus::Switching
            | ManagedCodexTaskStatus::Resuming
    )
}

async fn initialize_and_recover(app: &AppHandle) -> Result<(), String> {
    let store = store()?;
    store.initialize()?;
    let tasks = store.list_tasks(Some(MAX_TASK_LIST_LIMIT))?;
    for mut task in tasks {
        if !task.status.is_runtime_active() {
            continue;
        }

        match recorded_process_state(&task) {
            RecordedProcessState::MatchingAlive => {
                task.mark_needs_attention(
                    "Cockpit restarted while the matching managed Codex process was still alive. Its stdout cannot be reattached safely; terminate the orphan process before resuming.",
                );
                store.save_task(&task)?;
                emit_task_updated(app, &task);
                continue;
            }
            RecordedProcessState::PidReused | RecordedProcessState::NotRunning => {}
        }

        if task.recovery_attempts >= 1 {
            cleanup_task_credentials(&task).await;
            task.clear_process();
            task.mark_needs_attention(
                "automatic crash recovery was already attempted once for this task",
            );
            store.save_task(&task)?;
            emit_task_updated(app, &task);
            continue;
        }

        let Some(thread_id) = task.thread_id.clone() else {
            cleanup_task_credentials(&task).await;
            task.clear_process();
            task.mark_needs_attention(
                "Cockpit restarted before Codex reported a thread id; automatic recovery is unsafe",
            );
            store.save_task(&task)?;
            emit_task_updated(app, &task);
            continue;
        };
        let (task_home, _) = task_directories(&task.id)?;
        if let Some(account_id) = task.active_account_id.clone() {
            let _ = codex_account::sync_managed_projection_from_auth_dir(&account_id, &task_home);
            if !task_home.join("auth.json").is_file() {
                if let Err(error) = codex_account::prepare_account_for_injection_from_auth_dir(
                    &account_id,
                    Some(&task_home),
                )
                .await
                {
                    cleanup_task_credentials(&task).await;
                    task.clear_process();
                    task.mark_needs_attention(format!(
                        "could not prepare credentials for App Server recovery verification: {}",
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ));
                    store.save_task(&task)?;
                    emit_task_updated(app, &task);
                    continue;
                }
            }
        }

        let recovery = verify_thread_with_app_server(&task, &task_home, &thread_id).await;
        if let Some(account_id) = task.active_account_id.clone() {
            let _ = codex_account::sync_managed_projection_from_auth_dir(&account_id, &task_home);
        }
        let _ = codex_account::cleanup_managed_auth_dir(&task_home);
        task.clear_process();

        match recovery {
            Ok(AppServerRecoveryResult::Completed(evidence)) => {
                apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
            }
            Ok(AppServerRecoveryResult::QuotaFailed(evidence)) => {
                apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
                if task.register_single_recovery_failover().is_err() {
                    store.save_task(&task)?;
                    emit_task_updated(app, &task);
                }
            }
            Ok(AppServerRecoveryResult::NonQuotaFailed(evidence)) => {
                apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
            }
            Ok(AppServerRecoveryResult::Interrupted(evidence)) => {
                task.plan_single_recovery_resume_current()?;
                store.save_transition(&task, &evidence)?;
                emit_evidence(app, &task.id, &evidence);
                emit_task_updated(app, &task);
            }
            Ok(AppServerRecoveryResult::Ambiguous(reason)) | Err(reason) => {
                task.mark_needs_attention(format!(
                    "App Server recovery verification was inconclusive: {}",
                    sanitize_text(&reason, MAX_PERSISTED_ERROR_CHARS)
                ));
                persist_audit(
                    app,
                    &store,
                    &task,
                    "recovery.verification_inconclusive",
                    task.needs_attention_reason
                        .as_deref()
                        .unwrap_or("inconclusive"),
                )?;
            }
        }
    }
    Ok(())
}

fn recorded_process_state(task: &ManagedCodexTask) -> RecordedProcessState {
    let Some(pid) = task.process_id else {
        return RecordedProcessState::NotRunning;
    };
    if !process::is_pid_running(pid) {
        return RecordedProcessState::NotRunning;
    }
    let Some(expected_executable) = task.executable_path.as_deref() else {
        return RecordedProcessState::PidReused;
    };
    let Some(expected_started_at) = task.process_started_at else {
        return RecordedProcessState::PidReused;
    };

    let sys_pid = Pid::from(pid as usize);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sys_pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let Some(found) = system.process(sys_pid) else {
        return RecordedProcessState::NotRunning;
    };
    let executable_matches = found
        .exe()
        .map(|path| paths_refer_to_same_executable(path, Path::new(expected_executable)))
        .unwrap_or(false);
    let expected_seconds = expected_started_at / 1_000;
    let actual_seconds = found.start_time() as i64;
    let start_matches = (actual_seconds - expected_seconds).abs() <= 5;
    if executable_matches && start_matches {
        RecordedProcessState::MatchingAlive
    } else {
        RecordedProcessState::PidReused
    }
}

fn paths_refer_to_same_executable(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

async fn verify_thread_with_app_server(
    task: &ManagedCodexTask,
    task_home: &Path,
    thread_id: &str,
) -> Result<AppServerRecoveryResult, String> {
    let cli = codex_wakeup::resolve_cli_runtime()?;
    verify_thread_with_app_server_runtime(task, task_home, thread_id, &cli).await
}

async fn verify_thread_with_app_server_runtime(
    task: &ManagedCodexTask,
    task_home: &Path,
    thread_id: &str,
    cli: &codex_wakeup::CodexCliResolvedRuntime,
) -> Result<AppServerRecoveryResult, String> {
    let mut command = if let Some(node_path) = cli.node_path.as_deref() {
        let mut command = std::process::Command::new(node_path);
        command.arg(&cli.binary_path);
        command
    } else {
        std::process::Command::new(&cli.binary_path)
    };
    process::apply_managed_proxy_env_to_command(&mut command);
    command
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .env("CODEX_HOME", task_home)
        .current_dir(&task.config.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0200 | 0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = TokioCommand::from(command)
        .spawn()
        .map_err(|error| format!("launch Codex App Server failed: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex App Server stdin was not piped".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex App Server stdout was not piped".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Codex App Server stderr was not piped".to_string())?;
    let stderr_reader = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut summary = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if !summary.is_empty() {
                summary.push('\n');
            }
            summary.push_str(&sanitize_text(&line, MAX_STDERR_SUMMARY_CHARS));
            summary = keep_last_chars(&summary, MAX_STDERR_SUMMARY_CHARS);
        }
        summary
    });
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send_app_server_request(
            &mut stdin,
            serde_json::json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "cockpit-tools-managed-task-supervisor",
                        "title": "Cockpit Managed Task Supervisor",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
        )
        .await?;
        wait_for_app_server_response(&mut lines, 1).await?;
        send_app_server_request(
            &mut stdin,
            serde_json::json!({ "method": "initialized", "params": {} }),
        )
        .await?;
        send_app_server_request(
            &mut stdin,
            serde_json::json!({
                "method": "thread/read",
                "id": 2,
                "params": { "threadId": thread_id, "includeTurns": true }
            }),
        )
        .await?;
        let response = wait_for_app_server_response(&mut lines, 2).await?;
        classify_thread_read_recovery(task, thread_id, &response)
    }
    .await;

    drop(stdin);
    let _ = terminate_child_tree(&mut child).await;
    let stderr_summary = stderr_reader.await.unwrap_or_default();
    result.map_err(|error| {
        if stderr_summary.is_empty() {
            error
        } else {
            sanitize_text(
                &format!("{error}; stderr: {stderr_summary}"),
                MAX_PERSISTED_ERROR_CHARS,
            )
        }
    })
}

async fn send_app_server_request(
    stdin: &mut tokio::process::ChildStdin,
    request: Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&request)
        .map_err(|error| format!("encode App Server request failed: {error}"))?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .map_err(|error| format!("write App Server request failed: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("flush App Server request failed: {error}"))
}

async fn wait_for_app_server_response(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: i64,
) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let line = lines
                .next_line()
                .await
                .map_err(|error| format!("read App Server response failed: {error}"))?
                .ok_or_else(|| "App Server exited before returning a response".to_string())?;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id").and_then(Value::as_i64) != Some(request_id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!(
                    "App Server request {request_id} failed: {}",
                    sanitize_text(&error.to_string(), MAX_PERSISTED_ERROR_CHARS)
                ));
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| format!("App Server response {request_id} omitted result"));
        }
    })
    .await
    .map_err(|_| format!("App Server request {request_id} timed out"))?
}

fn classify_thread_read_recovery(
    task: &ManagedCodexTask,
    thread_id: &str,
    result: &Value,
) -> Result<AppServerRecoveryResult, String> {
    let thread = result
        .get("thread")
        .ok_or_else(|| "thread/read response omitted thread".to_string())?;
    if thread
        .pointer("/status/type")
        .and_then(Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("active"))
    {
        return Ok(AppServerRecoveryResult::Ambiguous(
            "thread/read still reports an active thread while no matching process is available"
                .to_string(),
        ));
    }
    let turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .ok_or_else(|| "thread/read response omitted turns".to_string())?;
    let Some(last_turn) = turns.last() else {
        return Ok(AppServerRecoveryResult::Ambiguous(
            "thread/read returned no turns".to_string(),
        ));
    };
    let status = last_turn
        .get("status")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .ok_or_else(|| "last App Server turn omitted status".to_string())?;

    let mut normalized_turn = last_turn.clone();
    if normalized_turn.get("error").is_none() {
        if let Some(codex_error_info) = normalized_turn.get("codexErrorInfo").cloned() {
            normalized_turn["error"] = serde_json::json!({
                "codexErrorInfo": codex_error_info
            });
        }
    }
    let mut evidence = classify_codex_event(
        CodexEventSource::AppServer,
        &serde_json::json!({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": normalized_turn
            }
        }),
    )
    .with_id(format!(
        "{}:{}:app_server_recovery",
        task.id, task.run_generation
    ));
    evidence.message = evidence
        .message
        .as_deref()
        .map(|message| sanitize_text(message, MAX_PERSISTED_ERROR_CHARS));
    match status.as_str() {
        "completed" => Ok(AppServerRecoveryResult::Completed(evidence)),
        "failed" if evidence.confirms_quota_exhaustion() => {
            Ok(AppServerRecoveryResult::QuotaFailed(evidence))
        }
        "failed" => Ok(AppServerRecoveryResult::NonQuotaFailed(evidence)),
        "interrupted" | "cancelled" | "canceled" => {
            Ok(AppServerRecoveryResult::Interrupted(evidence))
        }
        _ => Ok(AppServerRecoveryResult::Ambiguous(format!(
            "unsupported last turn status: {status}"
        ))),
    }
}

async fn scheduler_loop(app: AppHandle) {
    loop {
        if app_lifecycle::is_shutdown_started() {
            shutdown();
            return;
        }

        let next_task_id = match next_runnable_task_id() {
            Ok(value) => value,
            Err(error) => {
                logger::log_error(&format!(
                    "[ManagedCodexTask] queue read failed: {}",
                    sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                ));
                None
            }
        };

        let Some(task_id) = next_task_id else {
            tokio::select! {
                _ = runtime_notify().notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
            continue;
        };

        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut state = runtime_state().lock().await;
            if state.active_task_id.is_some() {
                continue;
            }
            state.active_task_id = Some(task_id.clone());
            state
                .cancel_flags
                .insert(task_id.clone(), cancel_flag.clone());
        }

        if let Err(error) = execute_task(&app, &task_id, cancel_flag).await {
            logger::log_error(&format!(
                "[ManagedCodexTask] task={} runner failed: {}",
                task_id,
                sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
            ));
            if let Ok(store) = store() {
                if let Ok(Some(mut task)) = store.get_task(&task_id) {
                    cleanup_task_credentials(&task).await;
                    task.clear_process();
                    task.mark_needs_attention(format!(
                        "managed task runner stopped unexpectedly: {}",
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ));
                    let _ = store.save_task(&task);
                    emit_task_updated(&app, &task);
                }
            }
        }

        {
            let mut state = runtime_state().lock().await;
            state.cancel_flags.remove(&task_id);
            if state.active_task_id.as_deref() == Some(task_id.as_str()) {
                state.active_task_id = None;
            }
        }
        runtime_notify().notify_one();
    }
}

fn next_runnable_task_id() -> Result<Option<String>, String> {
    let mut tasks = store()?
        .list_tasks(Some(MAX_TASK_LIST_LIMIT))?
        .into_iter()
        .filter(|task| is_scheduler_runnable(task.status))
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(tasks.first().map(|task| task.id.clone()))
}

async fn execute_task(
    app: &AppHandle,
    task_id: &str,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let store = store()?;
    let mut task = store
        .get_task(task_id)?
        .ok_or_else(|| format!("managed Codex task does not exist: {task_id}"))?;
    let (task_home, workspace_meta) = task_directories(&task.id)?;
    fs::create_dir_all(&task_home).map_err(|error| {
        format!(
            "create managed task CODEX_HOME failed ({}): {error}",
            task_home.display()
        )
    })?;
    fs::create_dir_all(&workspace_meta).map_err(|error| {
        format!(
            "create managed task metadata directory failed ({}): {error}",
            workspace_meta.display()
        )
    })?;

    let mut locally_rejected = HashSet::new();
    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            cleanup_task_credentials(&task).await;
            task.mark_cancelled("managed Codex task was cancelled before process launch");
            store.save_task(&task)?;
            emit_task_updated(app, &task);
            return Ok(());
        }

        let launch_mode = match task.status {
            ManagedCodexTaskStatus::Queued => {
                let Some(account_id) = select_and_prepare_initial_account(
                    app,
                    &store,
                    &mut task,
                    &task_home,
                    &mut locally_rejected,
                )
                .await?
                else {
                    return Ok(());
                };
                logger::log_info(&format!(
                    "[ManagedCodexTask] task={} prepared initial account={}",
                    task.id,
                    masked_account_id(&account_id)
                ));
                ProcessLaunchMode::Initial {
                    objective: task.config.objective.clone(),
                }
            }
            ManagedCodexTaskStatus::Switching => {
                let Some((thread_id, prompt)) = select_and_prepare_resume_account(
                    app,
                    &store,
                    &mut task,
                    &task_home,
                    &mut locally_rejected,
                )
                .await?
                else {
                    return Ok(());
                };
                ProcessLaunchMode::Resume { thread_id, prompt }
            }
            ManagedCodexTaskStatus::Resuming => {
                let account_id = task
                    .active_account_id
                    .clone()
                    .ok_or_else(|| "managed task has no account for resume".to_string())?;
                if let Err(error) = codex_account::prepare_account_for_injection_from_auth_dir(
                    &account_id,
                    Some(&task_home),
                )
                .await
                {
                    cleanup_task_credentials(&task).await;
                    task.mark_needs_attention(format!(
                        "could not refresh the current account for resume: {}",
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ));
                    persist_audit(
                        app,
                        &store,
                        &task,
                        "account.resume_preparation_failed",
                        &format!(
                            "account={} error={}",
                            masked_account_id(&account_id),
                            sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                        ),
                    )?;
                    return Ok(());
                }
                let thread_id = task
                    .thread_id
                    .clone()
                    .ok_or_else(|| "managed task has no Codex thread id for resume".to_string())?;
                ProcessLaunchMode::Resume {
                    thread_id,
                    prompt: task.continuation_prompt(),
                }
            }
            _ => return Ok(()),
        };

        let outcome = match run_codex_process(
            app,
            &store,
            &mut task,
            &task_home,
            launch_mode,
            cancel_flag.clone(),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                cleanup_task_credentials(&task).await;
                task.clear_process();
                let mut evidence = CodexTaskEvidence::confirmed_exec_exit(None, None)
                    .with_id(format!("{}:{}:launch_failed", task.id, task.run_generation));
                evidence.message = Some(sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS));
                apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
                return Ok(());
            }
        };

        sync_and_cleanup_task_credentials(app, &store, &mut task, &task_home).await;
        task.clear_process();
        store.save_task(&task)?;
        emit_task_updated(app, &task);

        if outcome.malformed_line_count > 0 {
            persist_audit(
                app,
                &store,
                &task,
                "exec_json.malformed_lines",
                &format!(
                    "ignored {} non-empty stdout line(s) that were not valid JSON",
                    outcome.malformed_line_count
                ),
            )?;
        }

        if outcome.cancelled && !outcome.saw_terminal {
            let evidence = CodexTaskEvidence::confirmed_user_interruption(task.thread_id.clone())
                .with_id(format!("{}:{}:cancelled", task.id, task.run_generation));
            apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
            return Ok(());
        }

        if !outcome.saw_terminal {
            if outcome.status.as_ref().is_some_and(ExitStatus::success) {
                task.mark_needs_attention(
                    "Codex exec exited successfully without an authoritative turn.completed or turn.failed event",
                );
                persist_audit(
                    app,
                    &store,
                    &task,
                    "process.exit_without_terminal",
                    task.needs_attention_reason
                        .as_deref()
                        .unwrap_or("missing terminal event"),
                )?;
                return Ok(());
            }
            let mut evidence = CodexTaskEvidence::confirmed_exec_exit(
                outcome.last_structured_error.as_ref(),
                outcome.status.as_ref().and_then(ExitStatus::code),
            )
            .with_id(format!("{}:{}:process_exit", task.id, task.run_generation));
            if evidence.failure_class == Some(CodexFailureClass::Other)
                && !outcome.stderr_summary.is_empty()
            {
                evidence.message = Some(sanitize_text(
                    &format!(
                        "{}; stderr: {}",
                        evidence
                            .message
                            .as_deref()
                            .unwrap_or("Codex exec exited without a terminal event"),
                        outcome.stderr_summary
                    ),
                    MAX_PERSISTED_ERROR_CHARS,
                ));
            }
            apply_and_persist_evidence(app, &store, &mut task, evidence, true)?;
        }

        match task.status {
            ManagedCodexTaskStatus::Switching => {
                if task.thread_id.is_none() {
                    task.mark_needs_attention(
                        "Codex reported a usage-limit terminal state without a thread id; automatic resume is unsafe",
                    );
                    store.save_task(&task)?;
                    emit_task_updated(app, &task);
                    return Ok(());
                }
                if let Some(account_id) = task.active_account_id.clone() {
                    codex_local_access::mark_managed_task_account_quota_exhausted(
                        &account_id,
                        task.config.model.as_deref(),
                    )
                    .await;
                }
                // The process is fully reaped and credentials are cleared at this point. Only
                // now may the selector inject another account and resume the original thread.
                continue;
            }
            ManagedCodexTaskStatus::Completed => {
                if let Some(account_id) = task.active_account_id.as_deref() {
                    codex_local_access::mark_managed_task_account_success(account_id).await;
                }
                return Ok(());
            }
            ManagedCodexTaskStatus::Failed
            | ManagedCodexTaskStatus::Cancelled
            | ManagedCodexTaskStatus::NeedsAttention => return Ok(()),
            _ => {
                task.mark_needs_attention(
                    "Codex process exited without a final supervisor disposition",
                );
                store.save_task(&task)?;
                emit_task_updated(app, &task);
                return Ok(());
            }
        }
    }
}

async fn select_and_prepare_initial_account(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    task_home: &Path,
    locally_rejected: &mut HashSet<String>,
) -> Result<Option<String>, String> {
    loop {
        let mut excluded = task.attempted_account_ids.clone();
        excluded.extend(locally_rejected.iter().cloned());
        let selection =
            select_account(task, &excluded, task.config.initial_account_id.as_deref()).await?;
        let Some(account_id) = selection.account_id.clone() else {
            cleanup_task_credentials(task).await;
            task.mark_needs_attention(selection.reason.clone());
            persist_audit(
                app,
                store,
                task,
                "account.selection_exhausted",
                &selection_audit_message(&selection),
            )?;
            return Ok(None);
        };

        match codex_account::prepare_account_for_injection_from_auth_dir(
            &account_id,
            Some(task_home),
        )
        .await
        {
            Ok(_) => {
                task.mark_preparing(&account_id)?;
                persist_audit(
                    app,
                    store,
                    task,
                    "account.selected",
                    &selection_audit_message(&selection),
                )?;
                return Ok(Some(account_id));
            }
            Err(error) => {
                let _ = codex_account::cleanup_managed_auth_dir(task_home);
                locally_rejected.insert(account_id.clone());
                persist_audit(
                    app,
                    store,
                    task,
                    "account.preparation_failed",
                    &format!(
                        "account={} error={}",
                        masked_account_id(&account_id),
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ),
                )?;
            }
        }
    }
}

async fn select_and_prepare_resume_account(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    task_home: &Path,
    locally_rejected: &mut HashSet<String>,
) -> Result<Option<(String, String)>, String> {
    loop {
        let mut excluded = task.attempted_account_ids.clone();
        excluded.extend(locally_rejected.iter().cloned());
        let selection = select_account(task, &excluded, None).await?;
        let Some(account_id) = selection.account_id.clone() else {
            cleanup_task_credentials(task).await;
            task.mark_needs_attention(selection.reason.clone());
            persist_audit(
                app,
                store,
                task,
                "account.selection_exhausted",
                &selection_audit_message(&selection),
            )?;
            return Ok(None);
        };

        task.mark_account_selected(&account_id)?;
        store.save_task(task)?;
        emit_task_updated(app, task);
        match codex_account::prepare_account_for_injection_from_auth_dir(
            &account_id,
            Some(task_home),
        )
        .await
        {
            Ok(_) => {
                let actions = task.mark_account_switched(&account_id)?;
                let resume = actions.into_iter().find_map(|action| match action {
                    CodexSupervisorAction::ResumeThread { thread_id, prompt } => {
                        Some((thread_id, prompt))
                    }
                    _ => None,
                });
                persist_audit(
                    app,
                    store,
                    task,
                    "account.switched",
                    &selection_audit_message(&selection),
                )?;
                return resume
                    .map(Some)
                    .ok_or_else(|| "account switch did not produce a resume action".to_string());
            }
            Err(error) => {
                let _ = codex_account::cleanup_managed_auth_dir(task_home);
                locally_rejected.insert(account_id.clone());
                task.reject_pending_account(
                    &account_id,
                    format!(
                        "selected account could not be refreshed for injection: {}",
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ),
                )?;
                persist_audit(
                    app,
                    store,
                    task,
                    "account.preparation_failed",
                    &format!(
                        "account={} error={}",
                        masked_account_id(&account_id),
                        sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                    ),
                )?;
            }
        }
    }
}

async fn select_account(
    task: &ManagedCodexTask,
    excluded: &[String],
    preferred: Option<&str>,
) -> Result<codex_local_access::ManagedTaskAccountSelection, String> {
    let selected = match &task.config.account_scope {
        ManagedCodexAccountScope::CockpitPool => None,
        ManagedCodexAccountScope::Selected { account_ids } => Some(account_ids.as_slice()),
    };
    codex_local_access::select_managed_task_account(
        selected,
        excluded,
        preferred,
        task.config.model.as_deref(),
    )
    .await
}

fn selection_audit_message(selection: &codex_local_access::ManagedTaskAccountSelection) -> String {
    let skipped = selection
        .skipped
        .iter()
        .take(32)
        .map(|item| format!("{}:{}", masked_account_id(&item.account_id), item.reason))
        .collect::<Vec<_>>()
        .join(", ");
    sanitize_text(
        &format!(
            "{}{}",
            selection.reason,
            if skipped.is_empty() {
                String::new()
            } else {
                format!("; skipped=[{skipped}]")
            }
        ),
        MAX_PERSISTED_ERROR_CHARS,
    )
}

async fn run_codex_process(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    task_home: &Path,
    launch_mode: ProcessLaunchMode,
    cancel_flag: Arc<AtomicBool>,
) -> Result<ProcessOutcome, String> {
    crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &task_home.join("config.toml"),
    )?;
    let cli = codex_wakeup::resolve_cli_runtime()?;
    let executable_identity = cli
        .node_path
        .clone()
        .unwrap_or_else(|| cli.binary_path.clone());
    let mut command = build_exec_command(&cli, task, task_home, &launch_mode)?;
    let mut child = command.spawn().map_err(|error| {
        format!(
            "launch Codex CLI failed ({}): {}",
            cli.binary_path,
            sanitize_text(&error.to_string(), MAX_PERSISTED_ERROR_CHARS)
        )
    })?;
    let pid = child
        .id()
        .ok_or_else(|| "Codex CLI started without a process id".to_string())?;
    task.mark_process_started(pid, executable_identity);
    store.save_task(task)?;
    emit_task_updated(app, task);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex CLI stdout was not piped".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Codex CLI stderr was not piped".to_string())?;
    let (stdout_tx, mut stdout_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let stdout_reader = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if stdout_tx.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_reader = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut summary = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            if !summary.is_empty() {
                summary.push('\n');
            }
            summary.push_str(&sanitize_text(&line, MAX_STDERR_SUMMARY_CHARS));
            summary = keep_last_chars(&summary, MAX_STDERR_SUMMARY_CHARS);
        }
        summary
    });

    let mut cancelled = false;
    let mut saw_terminal = false;
    let mut last_structured_error = None;
    let mut malformed_line_count = 0_u64;
    let status = loop {
        while let Ok(line) = stdout_rx.try_recv() {
            consume_exec_json_line(
                app,
                store,
                task,
                &line,
                &mut saw_terminal,
                &mut last_structured_error,
                &mut malformed_line_count,
            )?;
        }

        if cancel_flag.load(Ordering::SeqCst) {
            cancelled = true;
            break terminate_child_tree(&mut child).await;
        }

        match child.try_wait() {
            Ok(Some(exit_status)) => {
                break Some(exit_status);
            }
            Ok(None) => tokio::time::sleep(PROCESS_POLL_INTERVAL).await,
            Err(error) => {
                let _ = terminate_child_tree(&mut child).await;
                return Err(format!("wait for Codex CLI failed: {error}"));
            }
        }
    };

    let _ = stdout_reader.await;
    while let Ok(line) = stdout_rx.try_recv() {
        consume_exec_json_line(
            app,
            store,
            task,
            &line,
            &mut saw_terminal,
            &mut last_structured_error,
            &mut malformed_line_count,
        )?;
    }
    let stderr_summary = stderr_reader.await.unwrap_or_else(|_| String::new());

    Ok(ProcessOutcome {
        status,
        cancelled,
        saw_terminal,
        last_structured_error,
        stderr_summary,
        malformed_line_count,
    })
}

fn build_exec_command(
    cli: &codex_wakeup::CodexCliResolvedRuntime,
    task: &ManagedCodexTask,
    task_home: &Path,
    launch_mode: &ProcessLaunchMode,
) -> Result<TokioCommand, String> {
    let cwd = PathBuf::from(&task.config.cwd);
    if !cwd.is_dir() {
        return Err(format!(
            "managed Codex task working directory is unavailable: {}",
            cwd.display()
        ));
    }

    let mut command = if let Some(node_path) = cli.node_path.as_deref() {
        let mut command = std::process::Command::new(node_path);
        command.arg(&cli.binary_path);
        command
    } else {
        std::process::Command::new(&cli.binary_path)
    };
    process::apply_managed_proxy_env_to_command(&mut command);
    command
        .env("CODEX_HOME", task_home)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("--ask-for-approval")
        .arg("never")
        .arg("exec")
        .arg("--json")
        .arg("--sandbox")
        .arg("workspace-write")
        .arg("--skip-git-repo-check")
        .arg("--color")
        .arg("never")
        .arg("-C")
        .arg(&cwd);

    if let Some(model) = task.config.model.as_deref() {
        command
            .arg("-c")
            .arg(format!(r#"model="{}""#, escape_toml_basic_string(model)));
    }
    if let Some(reasoning_effort) = task.config.reasoning_effort.as_deref() {
        command.arg("-c").arg(format!(
            r#"model_reasoning_effort="{}""#,
            escape_toml_basic_string(reasoning_effort)
        ));
    }
    #[cfg(windows)]
    command.arg("-c").arg(r#"windows.sandbox="unelevated""#);
    match launch_mode {
        ProcessLaunchMode::Initial { objective } => {
            command.arg(objective);
        }
        ProcessLaunchMode::Resume { thread_id, prompt } => {
            command.arg("resume").arg(thread_id).arg(prompt);
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    Ok(TokioCommand::from(command))
}

fn consume_exec_json_line(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    line: &str,
    saw_terminal: &mut bool,
    last_structured_error: &mut Option<CodexTaskEvidence>,
    malformed_line_count: &mut u64,
) -> Result<(), String> {
    if line.trim().is_empty() {
        return Ok(());
    }
    task.last_event_seq = task.last_event_seq.saturating_add(1);
    let Ok(payload) = serde_json::from_str::<Value>(line) else {
        *malformed_line_count = malformed_line_count.saturating_add(1);
        task.last_activity_at = Some(chrono::Utc::now().timestamp_millis());
        task.updated_at = task.last_activity_at.unwrap_or(task.updated_at);
        store.save_task(task)?;
        emit_task_updated(app, task);
        return Ok(());
    };

    let mut evidence = classify_codex_event(CodexEventSource::ExecJson, &payload).with_id(format!(
        "{}:{}:{}",
        task.id, task.run_generation, task.last_event_seq
    ));
    evidence.message = evidence
        .message
        .as_deref()
        .map(|message| sanitize_text(message, MAX_PERSISTED_ERROR_CHARS));
    if evidence.terminal {
        *saw_terminal = true;
    }
    if matches!(
        evidence.kind,
        CodexEvidenceKind::QuotaWarning | CodexEvidenceKind::TurnFailed
    ) {
        *last_structured_error = Some(evidence.clone());
    }
    let persist = !matches!(
        evidence.kind,
        CodexEvidenceKind::Activity | CodexEvidenceKind::Unknown
    );
    apply_and_persist_evidence(app, store, task, evidence, persist)
}

fn apply_and_persist_evidence(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    evidence: CodexTaskEvidence,
    persist_evidence: bool,
) -> Result<(), String> {
    task.apply_evidence(&evidence);
    if persist_evidence {
        store.save_transition(task, &evidence)?;
        emit_evidence(app, &task.id, &evidence);
    } else {
        store.save_task(task)?;
    }
    emit_task_updated(app, task);
    Ok(())
}

fn persist_audit(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &ManagedCodexTask,
    event_type: &str,
    message: &str,
) -> Result<(), String> {
    let mut evidence = CodexTaskEvidence::activity(task.thread_id.clone(), event_type)
        .with_id(format!("{}:audit:{}", task.id, uuid::Uuid::new_v4()));
    evidence.message = Some(sanitize_text(message, MAX_PERSISTED_ERROR_CHARS));
    store.save_transition(task, &evidence)?;
    emit_task_updated(app, task);
    emit_evidence(app, &task.id, &evidence);
    Ok(())
}

async fn sync_and_cleanup_task_credentials(
    app: &AppHandle,
    store: &codex_task_store::CodexTaskStore,
    task: &mut ManagedCodexTask,
    task_home: &Path,
) {
    if let Some(account_id) = task.active_account_id.clone() {
        if let Err(error) =
            codex_account::sync_managed_projection_from_auth_dir(&account_id, task_home)
        {
            let _ = persist_audit(
                app,
                store,
                task,
                "account.token_sync_warning",
                &format!(
                    "account={} error={}",
                    masked_account_id(&account_id),
                    sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS)
                ),
            );
        }
    }
    if let Err(error) = codex_account::cleanup_managed_auth_dir(task_home) {
        let _ = persist_audit(
            app,
            store,
            task,
            "account.credential_cleanup_warning",
            &sanitize_text(&error, MAX_PERSISTED_ERROR_CHARS),
        );
    }
}

async fn cleanup_task_credentials(task: &ManagedCodexTask) {
    if let Ok((task_home, _)) = task_directories(&task.id) {
        let _ = codex_account::cleanup_managed_auth_dir(&task_home);
    }
}

fn task_directories(task_id: &str) -> Result<(PathBuf, PathBuf), String> {
    let root = codex_task_store::managed_tasks_root()?.join(task_id.trim());
    Ok((root.join("home"), root.join("workspace-meta")))
}

async fn terminate_child_tree(child: &mut Child) -> Option<ExitStatus> {
    let pid = child.id();
    if let Some(pid) = pid {
        let _ = terminate_pid_tree(pid).await;
    }
    if let Ok(Some(status)) = child.try_wait() {
        return Some(status);
    }
    let _ = child.kill().await;
    child.wait().await.ok()
}

async fn terminate_pid_tree(pid: u32) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = TokioCommand::from({
            let mut command = std::process::Command::new("taskkill");
            command
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        })
        .status()
        .await
        .map_err(|error| format!("terminate Codex process tree failed: {error}"))?;
        if !output.success() && process::is_pid_running(pid) {
            return Err(format!(
                "terminate Codex process tree failed: taskkill exited with {output}"
            ));
        }
    }
    #[cfg(unix)]
    {
        let process_group = -(pid as i32);
        unsafe {
            libc::kill(process_group, libc::SIGTERM);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        if process::is_pid_running(pid) {
            unsafe {
                libc::kill(process_group, libc::SIGKILL);
            }
        }
    }
    Ok(())
}

fn emit_task_updated(app: &AppHandle, task: &ManagedCodexTask) {
    if let Err(error) = app.emit(UPDATED_EVENT, task.clone()) {
        logger::log_warn(&format!(
            "[ManagedCodexTask] task={} update event failed: {}",
            task.id, error
        ));
    }
}

fn emit_evidence(app: &AppHandle, task_id: &str, evidence: &CodexTaskEvidence) {
    let payload = ManagedCodexEvidenceEventPayload {
        task_id: task_id.to_string(),
        evidence: evidence.clone(),
    };
    if let Err(error) = app.emit(EVIDENCE_EVENT, payload) {
        logger::log_warn(&format!(
            "[ManagedCodexTask] task={} evidence event failed: {}",
            task_id, error
        ));
    }
}

fn sanitize_text(value: &str, max_chars: usize) -> String {
    static BEARER_RE: OnceLock<Regex> = OnceLock::new();
    static SECRET_RE: OnceLock<Regex> = OnceLock::new();
    let bearer_re = BEARER_RE.get_or_init(|| {
        Regex::new(r"(?i)bearer\s+[a-z0-9._~+\-/=]+").expect("managed task bearer redaction regex")
    });
    let secret_re = SECRET_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(access_token|refresh_token|id_token|api[_-]?key|authorization)(\s*[:=]\s*)([^\s,}\]]+)"#,
        )
        .expect("managed task secret redaction regex")
    });
    let redacted = bearer_re.replace_all(value, "Bearer [REDACTED]");
    let redacted = secret_re.replace_all(&redacted, "$1$2[REDACTED]");
    redacted.chars().take(max_chars).collect()
}

fn keep_last_chars(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    value.chars().skip(count - max_chars).collect()
}

fn masked_account_id(account_id: &str) -> String {
    let account_id = account_id.trim();
    if account_id.chars().count() <= 8 {
        return "***".to_string();
    }
    let prefix = account_id.chars().take(4).collect::<String>();
    let suffix = account_id
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_task(cwd: &Path) -> ManagedCodexTask {
        let mut task = ManagedCodexTask::create(ManagedCodexTaskConfig {
            objective: "create the marker file".to_string(),
            cwd: cwd.display().to_string(),
            account_scope: ManagedCodexAccountScope::Selected {
                account_ids: vec!["account-a".to_string(), "account-b".to_string()],
            },
            initial_account_id: Some("account-a".to_string()),
            model: Some("gpt-test".to_string()),
            reasoning_effort: Some("high".to_string()),
            max_switches: None,
        })
        .expect("create managed task");
        task.mark_preparing("account-a").expect("prepare task");
        task
    }

    fn fake_node_runtime(script: &Path) -> codex_wakeup::CodexCliResolvedRuntime {
        codex_wakeup::CodexCliResolvedRuntime {
            binary_path: script.display().to_string(),
            node_path: Some("node".to_string()),
            source: "managed-test".to_string(),
        }
    }

    #[test]
    fn redacts_bearer_and_token_fields() {
        let sanitized = sanitize_text(
            r#"Authorization: Bearer secret.token access_token=abc refresh_token: def"#,
            2_000,
        );
        assert!(!sanitized.contains("secret.token"));
        assert!(!sanitized.contains("=abc"));
        assert!(!sanitized.contains(": def"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn keeps_only_last_bounded_stderr_chars() {
        assert_eq!(keep_last_chars("abcdef", 3), "def");
        assert_eq!(keep_last_chars("你好世界", 2), "世界");
    }

    #[test]
    fn masks_account_identifiers() {
        assert_eq!(masked_account_id("1234567890"), "1234…7890");
        assert_eq!(masked_account_id("short"), "***");
    }

    #[tokio::test]
    async fn fake_cli_initial_and_resume_keep_thread_and_non_ascii_cwd() {
        let root = std::env::temp_dir().join(format!(
            "cockpit managed runtime 空格 {}",
            uuid::Uuid::new_v4()
        ));
        let cwd = root.join("工作目录");
        let home = root.join("task home");
        fs::create_dir_all(&cwd).expect("create fake cwd");
        fs::create_dir_all(&home).expect("create fake home");
        let script = root.join("fake codex.js");
        let observed_home = root.join("observed-home.txt");
        let global_home = root.join("global home");
        let global_auth = global_home.join("auth.json");
        fs::create_dir_all(&global_home).expect("create synthetic global home");
        fs::write(&global_auth, b"synthetic-global-auth-must-not-change")
            .expect("write synthetic global auth");
        let global_auth_before = fs::read(&global_auth).expect("hash global auth before");
        fs::write(
            &script,
            r#"const fs = require('fs');
const path = require('path');
const args = process.argv.slice(2);
fs.writeFileSync(path.join(process.cwd(), args.includes('resume') ? 'resume-args.json' : 'initial-args.json'), JSON.stringify(args));
fs.writeFileSync(__OBSERVED_HOME__, process.env.CODEX_HOME || '');
console.log(JSON.stringify({type:'thread.started', thread_id:'thread-fixed'}));
console.log(JSON.stringify({type:'turn.started', thread_id:'thread-fixed', turn_id:'turn-1'}));
console.log(JSON.stringify({type:'item.completed', item:{type:'command_execution', text:'private transcript must not persist'}}));
console.log(JSON.stringify({type:'turn.completed', thread_id:'thread-fixed', turn_id:'turn-1'}));
"#
            .replace(
                "__OBSERVED_HOME__",
                &serde_json::to_string(&observed_home.display().to_string()).expect("home path"),
            ),
        )
        .expect("write fake CLI");

        let mut task = test_task(&cwd);
        let runtime = fake_node_runtime(&script);
        let initial = build_exec_command(
            &runtime,
            &task,
            &home,
            &ProcessLaunchMode::Initial {
                objective: task.config.objective.clone(),
            },
        )
        .expect("build initial command")
        .output()
        .await
        .expect("run initial command");
        assert!(initial.status.success());
        let stdout = String::from_utf8(initial.stdout).expect("utf8 stdout");
        assert!(stdout.contains("thread.started"));
        assert!(stdout.contains("turn.completed"));
        let initial_args: Vec<String> = serde_json::from_str(
            &fs::read_to_string(cwd.join("initial-args.json")).expect("read initial args"),
        )
        .expect("parse initial args");
        assert!(initial_args
            .windows(2)
            .any(|pair| pair == ["exec", "--json"]));
        assert!(initial_args
            .windows(3)
            .any(|args| args == ["--ask-for-approval", "never", "exec"]));
        assert!(initial_args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]));
        #[cfg(windows)]
        assert!(initial_args
            .windows(2)
            .any(|pair| pair == ["-c", r#"windows.sandbox="unelevated""#]));
        assert!(initial_args
            .iter()
            .any(|arg| arg == "create the marker file"));
        assert_eq!(
            fs::read_to_string(&observed_home).expect("read observed CODEX_HOME"),
            home.display().to_string()
        );

        task.thread_id = Some("thread-fixed".to_string());
        task.status = ManagedCodexTaskStatus::Resuming;
        let resumed = build_exec_command(
            &runtime,
            &task,
            &home,
            &ProcessLaunchMode::Resume {
                thread_id: "thread-fixed".to_string(),
                prompt: task.continuation_prompt(),
            },
        )
        .expect("build resume command")
        .output()
        .await
        .expect("run resume command");
        assert!(resumed.status.success());
        let resume_args: Vec<String> = serde_json::from_str(
            &fs::read_to_string(cwd.join("resume-args.json")).expect("read resume args"),
        )
        .expect("parse resume args");
        let resume_index = resume_args
            .iter()
            .position(|arg| arg == "resume")
            .expect("resume arg");
        assert_eq!(
            resume_args.get(resume_index + 1).map(String::as_str),
            Some("thread-fixed")
        );
        assert!(resume_args
            .get(resume_index + 2)
            .is_some_and(|prompt| prompt.contains("do not repeat completed steps")));
        assert_eq!(
            fs::read(&global_auth).expect("hash global auth after"),
            global_auth_before,
            "managed execution must not mutate the global auth file"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Manual, opt-in side-effect smoke test for an already-authorized Cockpit OAuth account.
    ///
    /// This test is deliberately ignored by default and additionally requires
    /// `COCKPIT_MANAGED_REAL_SMOKE=1`. It creates and removes one isolated directory below the
    /// system temp directory, never writes to the repository, and never projects credentials to
    /// the user's global Codex home.
    #[tokio::test]
    #[ignore = "requires a real authorized Cockpit OAuth account and the official Codex CLI"]
    async fn real_managed_cli_smoke_uses_isolated_cockpit_projection() -> Result<(), String> {
        if std::env::var("COCKPIT_MANAGED_REAL_SMOKE").as_deref() != Ok("1") {
            return Err(
                "set COCKPIT_MANAGED_REAL_SMOKE=1 to acknowledge the real CLI side effect"
                    .to_string(),
            );
        }

        struct RealSmokeCleanup {
            root: PathBuf,
            task_home: PathBuf,
        }

        impl Drop for RealSmokeCleanup {
            fn drop(&mut self) {
                let _ = codex_account::cleanup_managed_auth_dir(&self.task_home);
                let temp_root = std::env::temp_dir();
                let is_expected_root = self.root.starts_with(&temp_root)
                    && self
                        .root
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.starts_with("cockpit-managed-real-smoke-"));
                if is_expected_root {
                    let _ = fs::remove_dir_all(&self.root);
                }
            }
        }

        fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
            match fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("read {} failed: {error}", path.display())),
            }
        }

        fn current_account_id(index_path: &Path) -> Result<Option<String>, String> {
            let payload = fs::read_to_string(index_path)
                .map_err(|error| format!("read Cockpit account index failed: {error}"))?;
            let value: Value = serde_json::from_str(&payload)
                .map_err(|error| format!("parse Cockpit account index failed: {error}"))?;
            Ok(value
                .get("current_account_id")
                .and_then(Value::as_str)
                .map(str::to_string))
        }

        struct RealSmokeOutputSummary {
            thread_ids: Vec<String>,
            terminal_success: bool,
            terminal_quota: bool,
            terminal_failure: Option<String>,
            completed_item_summaries: Vec<String>,
        }

        fn summarize_real_smoke_output(output: &std::process::Output) -> RealSmokeOutputSummary {
            let mut summary = RealSmokeOutputSummary {
                thread_ids: Vec::new(),
                terminal_success: false,
                terminal_quota: false,
                terminal_failure: None,
                completed_item_summaries: Vec::new(),
            };
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let Ok(payload) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) == Some("item.completed")
                    && summary.completed_item_summaries.len() < 16
                {
                    let item = payload.get("item").unwrap_or(&Value::Null);
                    let item_type = item
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let item_summary = match item_type {
                        "agent_message" => format!(
                            "agent_message:{}",
                            sanitize_text(
                                item.get("text").and_then(Value::as_str).unwrap_or_default(),
                                800,
                            )
                        ),
                        "command_execution" => format!(
                            "command_execution:status={},exitCode={}",
                            item.get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            item.get("exit_code")
                                .and_then(Value::as_i64)
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_string())
                        ),
                        "file_change" => format!(
                            "file_change:status={}",
                            item.get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                        ),
                        other => other.to_string(),
                    };
                    summary.completed_item_summaries.push(item_summary);
                }
                let evidence = classify_codex_event(CodexEventSource::ExecJson, &payload);
                if let Some(thread_id) = evidence.thread_id {
                    if !summary.thread_ids.contains(&thread_id) {
                        summary.thread_ids.push(thread_id);
                    }
                }
                if evidence.kind == CodexEvidenceKind::TurnCompleted && evidence.terminal {
                    summary.terminal_success = true;
                }
                if evidence.kind == CodexEvidenceKind::TurnFailed && evidence.terminal {
                    summary.terminal_quota =
                        evidence.failure_class == Some(CodexFailureClass::QuotaExhausted);
                    summary.terminal_failure = Some(format!(
                        "class={:?}, code={}",
                        evidence.failure_class,
                        evidence.error_code.as_deref().unwrap_or("unknown")
                    ));
                }
            }
            summary
        }

        let user_home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or_else(|| "user home is unavailable".to_string())?;
        let global_auth_path = user_home.join(".codex").join("auth.json");
        let global_config_path = user_home.join(".codex").join("config.toml");
        let cockpit_index_path = user_home
            .join(".antigravity_cockpit")
            .join("codex_accounts.json");
        let global_auth_before = read_optional_bytes(&global_auth_path)?;
        let global_config_before = read_optional_bytes(&global_config_path)?;
        let current_account_before = current_account_id(&cockpit_index_path)?;

        let selection =
            codex_local_access::select_managed_task_account(None, &[], None, None).await?;
        let account_id = selection.account_id.ok_or_else(|| {
            format!(
                "no eligible Cockpit OAuth account is available: {}",
                sanitize_text(&selection.reason, MAX_PERSISTED_ERROR_CHARS)
            )
        })?;

        let smoke_id = uuid::Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("cockpit-managed-real-smoke-{smoke_id}"));
        let cwd = root.join("isolated workspace 鐪熷疄");
        let task_home = root.join("home");
        let workspace_meta = root.join("workspace-meta");
        fs::create_dir_all(&cwd)
            .map_err(|error| format!("create isolated smoke workspace failed: {error}"))?;
        fs::create_dir_all(&task_home)
            .map_err(|error| format!("create isolated task home failed: {error}"))?;
        fs::create_dir_all(&workspace_meta)
            .map_err(|error| format!("create isolated workspace metadata failed: {error}"))?;
        let _cleanup = RealSmokeCleanup {
            root: root.clone(),
            task_home: task_home.clone(),
        };

        codex_account::prepare_account_for_injection_from_auth_dir(&account_id, Some(&task_home))
            .await?;
        if codex_account::read_managed_projection_account_id_from_dir(&task_home).as_deref()
            != Some(account_id.as_str())
        {
            return Err("Cockpit managed credential projection was not created".to_string());
        }

        let marker_name = format!("cockpit-managed-smoke-{smoke_id}.txt");
        let marker_path = cwd.join(&marker_name);
        let marker_contents = "cockpit-managed-real-smoke-ok";
        let objective = format!(
            "Create a UTF-8 text file named {marker_name} in the current working directory with exactly the single line `{marker_contents}`. Do not modify any other files. Verify the file exists, then finish the task."
        );
        let real_max_switches = std::env::var("COCKPIT_MANAGED_REAL_SMOKE_MAX_SWITCHES")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .map(|value| value.min(9))
            .unwrap_or(3);
        let mut task = ManagedCodexTask::create(ManagedCodexTaskConfig {
            objective: objective.clone(),
            cwd: cwd.display().to_string(),
            account_scope: ManagedCodexAccountScope::CockpitPool,
            initial_account_id: Some(account_id.clone()),
            model: None,
            reasoning_effort: None,
            max_switches: Some(real_max_switches),
        })?;
        task.mark_preparing(&account_id)?;

        let runtime = codex_wakeup::resolve_cli_runtime()?;
        let mut command = build_exec_command(
            &runtime,
            &task,
            &task_home,
            &ProcessLaunchMode::Initial { objective },
        )?;
        command.kill_on_drop(true);
        let output_result = tokio::time::timeout(Duration::from_secs(600), command.output())
            .await
            .map_err(|_| "real Codex CLI smoke test timed out after 600 seconds".to_string())?
            .map_err(|error| format!("real Codex CLI smoke launch failed: {error}"));

        let sync_result =
            codex_account::sync_managed_projection_from_auth_dir(&account_id, &task_home)
                .map(|_| ());
        let cleanup_result = codex_account::cleanup_managed_auth_dir(&task_home);
        let mut output = output_result?;
        sync_result?;
        cleanup_result?;

        let mut output_summary = summarize_real_smoke_output(&output);
        let mut active_account_id = account_id.clone();
        let mut switch_count = 0_u32;
        let mut thread_id = output_summary.thread_ids.first().cloned();
        let mut attempted_account_ids = vec![account_id.clone()];

        while (!output.status.success() || !output_summary.terminal_success)
            && output_summary.terminal_quota
            && switch_count < real_max_switches
        {
            let original_thread_id = thread_id.clone().ok_or_else(|| {
                "quota-exhausted real turn did not expose a thread identifier".to_string()
            })?;
            codex_local_access::mark_managed_task_account_quota_exhausted(&active_account_id, None)
                .await;
            let next_selection = codex_local_access::select_managed_task_account(
                None,
                &attempted_account_ids,
                None,
                None,
            )
            .await?;
            let next_account_id = next_selection.account_id.ok_or_else(|| {
                format!(
                    "a real account reached a quota terminal state after {} switch(es), but no next eligible account was available: {}",
                    switch_count,
                    sanitize_text(&next_selection.reason, MAX_PERSISTED_ERROR_CHARS)
                )
            })?;
            codex_account::prepare_account_for_injection_from_auth_dir(
                &next_account_id,
                Some(&task_home),
            )
            .await?;
            if codex_account::read_managed_projection_account_id_from_dir(&task_home).as_deref()
                != Some(next_account_id.as_str())
            {
                return Err("replacement Cockpit credential projection was not created".to_string());
            }

            task.active_account_id = Some(next_account_id.clone());
            task.status = ManagedCodexTaskStatus::Resuming;
            task.thread_id = Some(original_thread_id.clone());
            let mut resume_command = build_exec_command(
                &runtime,
                &task,
                &task_home,
                &ProcessLaunchMode::Resume {
                    thread_id: original_thread_id.clone(),
                    prompt: task.continuation_prompt(),
                },
            )?;
            resume_command.kill_on_drop(true);
            let resume_output_result =
                tokio::time::timeout(Duration::from_secs(600), resume_command.output())
                    .await
                    .map_err(|_| {
                        "real Codex CLI resume smoke timed out after 600 seconds".to_string()
                    })?
                    .map_err(|error| format!("real Codex CLI resume launch failed: {error}"));
            let resume_sync_result =
                codex_account::sync_managed_projection_from_auth_dir(&next_account_id, &task_home)
                    .map(|_| ());
            let resume_cleanup_result = codex_account::cleanup_managed_auth_dir(&task_home);
            output = resume_output_result?;
            resume_sync_result?;
            resume_cleanup_result?;
            output_summary = summarize_real_smoke_output(&output);
            if output_summary.thread_ids.is_empty()
                || output_summary
                    .thread_ids
                    .iter()
                    .any(|observed| observed != &original_thread_id)
            {
                return Err(format!(
                    "real resume did not preserve the original thread id: expected={}, observed={}",
                    sanitize_text(&original_thread_id, 200),
                    sanitize_text(&output_summary.thread_ids.join(","), 500)
                ));
            }
            thread_id = Some(original_thread_id);
            attempted_account_ids.push(next_account_id.clone());
            active_account_id = next_account_id;
            switch_count = switch_count.saturating_add(1);
        }

        if !output.status.success() || !output_summary.terminal_success {
            let stderr = sanitize_text(
                &String::from_utf8_lossy(&output.stderr),
                MAX_PERSISTED_ERROR_CHARS,
            );
            return Err(format!(
                "real Codex CLI did not complete successfully after {} switch(es): status={}, terminalFailure={}, completedItems={}, stderr={}",
                switch_count,
                output.status,
                output_summary.terminal_failure.as_deref().unwrap_or("none"),
                sanitize_text(&output_summary.completed_item_summaries.join(" | "), 2_000),
                stderr
            ));
        }
        let thread_id = thread_id.ok_or_else(|| {
            "real Codex CLI completed without a structured thread identifier".to_string()
        })?;
        let actual_marker = fs::read_to_string(&marker_path).map_err(|error| {
            format!(
                "real smoke marker was not created: {error}; completedItems={}",
                sanitize_text(&output_summary.completed_item_summaries.join(" | "), 2_000)
            )
        })?;
        if actual_marker.trim() != marker_contents {
            return Err("real smoke marker content did not match the requested value".to_string());
        }
        let workspace_entries = fs::read_dir(&cwd)
            .map_err(|error| format!("read isolated workspace failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("enumerate isolated workspace failed: {error}"))?;
        if workspace_entries.len() != 1
            || workspace_entries[0].file_name().to_string_lossy() != marker_name
        {
            return Err("real smoke task modified an unexpected workspace path".to_string());
        }
        if task_home.join("auth.json").exists()
            || codex_account::read_managed_projection_account_id_from_dir(&task_home).is_some()
        {
            return Err(
                "managed credentials remained after the real smoke process exited".to_string(),
            );
        }
        if read_optional_bytes(&global_auth_path)? != global_auth_before {
            return Err("global Codex auth.json changed during managed smoke test".to_string());
        }
        if read_optional_bytes(&global_config_path)? != global_config_before {
            return Err("global Codex config.toml changed during managed smoke test".to_string());
        }
        if current_account_id(&cockpit_index_path)? != current_account_before {
            return Err(
                "Cockpit default Codex account changed during managed smoke test".to_string(),
            );
        }

        let mut version_command = if let Some(node_path) = runtime.node_path.as_deref() {
            let mut command = TokioCommand::new(node_path);
            command.arg(&runtime.binary_path);
            command
        } else {
            TokioCommand::new(&runtime.binary_path)
        };
        let version = version_command
            .arg("--version")
            .output()
            .await
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "REAL_MANAGED_SMOKE cli={} initialAccount={} activeAccount={} threadId={} switches={} status=completed globalAuthUnchanged=true globalConfigUnchanged=true",
            sanitize_text(&version, 120),
            masked_account_id(&account_id),
            masked_account_id(&active_account_id),
            sanitize_text(&thread_id, 200),
            switch_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_terminates_fake_cli_process_tree() {
        let root = std::env::temp_dir().join(format!(
            "cockpit-managed-runtime-tree-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create root");
        let script = root.join("tree.js");
        let child_pid_file = root.join("child.pid");
        fs::write(
            &script,
            format!(
                r#"const fs = require('fs');
const {{spawn}} = require('child_process');
const child = spawn(process.execPath, ['-e', 'setInterval(() => {{}}, 1000)'], {{stdio:'ignore'}});
fs.writeFileSync({}, String(child.pid));
setInterval(() => {{}}, 1000);
"#,
                serde_json::to_string(&child_pid_file.display().to_string()).expect("pid path")
            ),
        )
        .expect("write tree fake CLI");
        let task = test_task(&root);
        let runtime = fake_node_runtime(&script);
        let mut child = build_exec_command(
            &runtime,
            &task,
            &root.join("home"),
            &ProcessLaunchMode::Initial {
                objective: task.config.objective.clone(),
            },
        )
        .expect("build tree command")
        .spawn()
        .expect("spawn tree command");
        for _ in 0..100 {
            if child_pid_file.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let child_pid = fs::read_to_string(&child_pid_file)
            .expect("read child pid")
            .parse::<u32>()
            .expect("parse child pid");
        assert!(process::is_pid_running(child_pid));
        let _ = terminate_child_tree(&mut child).await;
        for _ in 0..80 {
            if !process::is_pid_running(child_pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!process::is_pid_running(child_pid));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn app_server_recovery_normalizes_usage_limit_terminal() {
        let root = std::env::temp_dir();
        let mut task = test_task(&root);
        task.thread_id = Some("thread-fixed".to_string());
        let result = classify_thread_read_recovery(
            &task,
            "thread-fixed",
            &serde_json::json!({
                "thread": {
                    "id": "thread-fixed",
                    "status": { "type": "notLoaded" },
                    "turns": [{
                        "id": "turn-last",
                        "status": "failed",
                        "codexErrorInfo": { "UsageLimitExceeded": { "planType": "team" } }
                    }]
                }
            }),
        )
        .expect("classify recovery");
        assert!(matches!(result, AppServerRecoveryResult::QuotaFailed(_)));
    }

    #[tokio::test]
    async fn fake_app_server_requires_initialized_then_reads_quota_terminal() {
        let root = std::env::temp_dir().join(format!(
            "cockpit-managed-app-server-{}",
            uuid::Uuid::new_v4()
        ));
        let cwd = root.join("工作 directory");
        let home = root.join("task home");
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::create_dir_all(&home).expect("create home");
        let script = root.join("fake app server.js");
        fs::write(
            &script,
            r#"const readline = require('readline');
let initialized = false;
const lines = readline.createInterface({input: process.stdin});
const send = (value) => process.stdout.write(JSON.stringify(value) + '\n');
lines.on('line', (line) => {
  const message = JSON.parse(line);
  if (message.method === 'initialize') {
    send({id: message.id, result: {userAgent: 'fake'}});
    return;
  }
  if (message.method === 'initialized') {
    initialized = true;
    return;
  }
  if (message.method === 'thread/read') {
    if (!initialized) {
      send({id: message.id, error: {code: -32000, message: 'Not initialized'}});
      return;
    }
    send({id: message.id, result: {thread: {
      id: message.params.threadId,
      status: {type: 'notLoaded'},
      turns: [{
        id: 'turn-last',
        status: 'failed',
        error: {codexErrorInfo: {UsageLimitExceeded: {planType: 'team'}}}
      }]
    }}});
  }
});
"#,
        )
        .expect("write fake App Server");
        let mut task = test_task(&cwd);
        task.thread_id = Some("thread-fixed".to_string());
        let result = verify_thread_with_app_server_runtime(
            &task,
            &home,
            "thread-fixed",
            &fake_node_runtime(&script),
        )
        .await
        .expect("verify fake App Server");
        assert!(matches!(result, AppServerRecoveryResult::QuotaFailed(_)));
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn missing_or_nonexecutable_cli_fails_before_managed_work_starts() {
        let root = std::env::temp_dir().join(format!(
            "cockpit-managed-cli-errors-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create root");
        let task = test_task(&root);
        let missing = codex_wakeup::CodexCliResolvedRuntime {
            binary_path: root.join("missing-codex").display().to_string(),
            node_path: None,
            source: "managed-test".to_string(),
        };
        let missing_result = build_exec_command(
            &missing,
            &task,
            &root.join("home"),
            &ProcessLaunchMode::Initial {
                objective: task.config.objective.clone(),
            },
        )
        .expect("build missing command")
        .spawn();
        assert!(missing_result.is_err());

        let denied = codex_wakeup::CodexCliResolvedRuntime {
            binary_path: root.display().to_string(),
            node_path: None,
            source: "managed-test".to_string(),
        };
        let denied_result = build_exec_command(
            &denied,
            &task,
            &root.join("home"),
            &ProcessLaunchMode::Initial {
                objective: task.config.objective.clone(),
            },
        )
        .expect("build denied command")
        .spawn();
        assert!(denied_result.is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
