//! Codex 官方客户端临时登录（Official desktop login）。
//!
//! 复用多开实例里「空白实例」的启动方式：为一台一次性的临时 profile 打开官方 Codex
//! 客户端，由官方客户端完成真实登录。关闭临时客户端后读取最终凭据，导入成功才清理。
//! 临时目录固定在应用数据目录下（不注册为实例、不影响默认实例和多开实例）；
//! 导入失败保留凭据供重试，明确取消则丢弃；清理失败由后台巡检重试。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::models::codex::CodexAccount;
use crate::modules::{account, app_lifecycle, codex_account, codex_instance, logger, process};

/// 临时登录 profile 的根目录（应用数据目录下），目录名固定便于巡检。
const TEMP_LOGIN_ROOT_DIR_NAME: &str = "codex-temp-login";
/// 会话标记文件：用于区分「我们创建的临时 profile」和其它误入目录。
const SESSION_MARKER_FILE_NAME: &str = ".cockpit-temp-login.json";
/// 待清理清单：记录尚未完全清理的会话（含已解析的运行目录），供巡检重试。
const PENDING_CLEANUP_FILE_NAME: &str = ".pending-cleanup.json";
const PROGRESS_EVENT: &str = "codex:temp-login-progress";
/// 凭据探测间隔：只做低成本检查（auth.json / 钥匙串条目是否存在）。
const CREDENTIAL_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 等待用户在官方客户端完成登录的最长时间。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// 关闭官方客户端的最长等待时间。
const CLOSE_TIMEOUT_SECS: u64 = 20;
/// 残留清理巡检周期与启动延迟（避免与启动高峰抢资源）。
const CLEANUP_SWEEP_INTERVAL: Duration = Duration::from_secs(600);
const CLEANUP_STARTUP_DELAY: Duration = Duration::from_secs(8);

/// 当前进程内的临时登录会话。
struct SessionRuntime {
    profile_dir: PathBuf,
    cancel: Arc<AtomicBool>,
    finished: bool,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, SessionRuntime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLEANUP_LOOP_STARTED: AtomicBool = AtomicBool::new(false);

/// 前端拿到的会话句柄。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTempLoginSession {
    pub session_id: String,
    pub profile_dir: String,
}

/// 单条清理失败记录（保留原因，便于排查与下次重试）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTempLoginCleanupFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTempLoginCleanupReport {
    pub removed: Vec<String>,
    pub failed: Vec<CodexTempLoginCleanupFailure>,
}

enum SessionOutcome {
    CredentialsReady,
    Imported(CodexAccount),
    Cancelled,
    Failed(String),
}

fn lock_sessions() -> std::sync::MutexGuard<'static, HashMap<String, SessionRuntime>> {
    SESSIONS.lock().unwrap_or_else(|error| error.into_inner())
}

/// 临时登录 profile 根目录。
pub fn temp_login_root_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(TEMP_LOGIN_ROOT_DIR_NAME))
}

/// 是否存在未结束的会话（同一时间只处理一个账号）。
fn has_unfinished_session() -> bool {
    lock_sessions().values().any(|session| !session.finished)
}

fn session_profile_dir(session_id: &str) -> Option<PathBuf> {
    lock_sessions()
        .get(session_id)
        .map(|session| session.profile_dir.clone())
}

fn session_cancel_flag(session_id: &str) -> Option<Arc<AtomicBool>> {
    lock_sessions()
        .get(session_id)
        .map(|session| Arc::clone(&session.cancel))
}

fn mark_session_finished(session_id: &str) {
    if let Some(session) = lock_sessions().get_mut(session_id) {
        session.finished = true;
    }
}

fn forget_session(session_id: &str) {
    lock_sessions().remove(session_id);
}

fn is_active_session_dir(dir: &Path) -> bool {
    lock_sessions()
        .values()
        .any(|session| !session.finished && session.profile_dir == dir)
}

/// 待清理记录：即使 profile 目录已被删除，也能继续清理它对应的运行目录与钥匙串条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingCleanupEntry {
    session_id: String,
    profile_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_user_data_dir: Option<String>,
}

fn pending_cleanup_path() -> Result<PathBuf, String> {
    Ok(temp_login_root_dir()?.join(PENDING_CLEANUP_FILE_NAME))
}

fn read_pending_cleanup_entries() -> Vec<PendingCleanupEntry> {
    let Ok(path) = pending_cleanup_path() else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<PendingCleanupEntry>>(&raw).unwrap_or_default()
}

fn write_pending_cleanup_entries(entries: &[PendingCleanupEntry]) {
    let Ok(path) = pending_cleanup_path() else {
        return;
    };
    if entries.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(entries) {
        Ok(serialized) => {
            if let Err(error) = fs::write(&path, serialized) {
                logger::log_warn(&format!(
                    "[Codex临时登录] 写入待清理清单失败: path={}, error={}",
                    path.display(),
                    error
                ));
            }
        }
        Err(error) => logger::log_warn(&format!("[Codex临时登录] 序列化待清理清单失败: {}", error)),
    }
}

fn register_pending_cleanup(entry: PendingCleanupEntry) {
    let mut entries = read_pending_cleanup_entries();
    entries.retain(|item| item.session_id != entry.session_id);
    entries.push(entry);
    write_pending_cleanup_entries(&entries);
}

fn unregister_pending_cleanup(session_id: &str) {
    let mut entries = read_pending_cleanup_entries();
    let before = entries.len();
    entries.retain(|item| item.session_id != session_id);
    if entries.len() != before {
        write_pending_cleanup_entries(&entries);
    }
}

fn emit_progress(
    app: &AppHandle,
    session_id: &str,
    phase: &str,
    progress: u8,
    account: Option<&CodexAccount>,
    error: Option<&str>,
) {
    let payload = serde_json::json!({
        "sessionId": session_id,
        "phase": phase,
        "progress": progress,
        "accountId": account.map(|item| item.id.clone()),
        "email": account.map(|item| item.email.clone()),
        // 完成后直接回传账号快照，前端无需在刷新列表后再等待列表状态同步。
        "account": account,
        "error": error,
    });
    if let Err(err) = app.emit(PROGRESS_EVENT, payload) {
        logger::log_warn(&format!(
            "[Codex临时登录] 进度事件发送失败: session_id={}, phase={}, error={}",
            session_id, phase, err
        ));
    }
}

/// 开始一次官方登录：创建空白临时 profile 并打开官方客户端。
pub fn start(app: AppHandle) -> Result<CodexTempLoginSession, String> {
    if has_unfinished_session() {
        return Err("已有一个官方登录正在进行中，请先完成或取消本次登录。".to_string());
    }

    let root = temp_login_root_dir()?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let profile_dir = root.join(&session_id);
    prepare_blank_profile_dir(&profile_dir)?;
    write_session_marker(&profile_dir, &session_id)?;

    let mut sessions = lock_sessions();
    if sessions.values().any(|session| !session.finished) {
        return Err("CODEX_TEMP_LOGIN_BUSY".to_string());
    }
    sessions.insert(
        session_id.clone(),
        SessionRuntime {
            profile_dir: profile_dir.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            finished: false,
        },
    );
    drop(sessions);
    // 先登记待清理记录：即使运行过程中进程被杀，也能按记录清理运行目录与钥匙串条目。
    register_pending_cleanup(PendingCleanupEntry {
        session_id: session_id.clone(),
        profile_dir: profile_dir.to_string_lossy().to_string(),
        app_user_data_dir: app_user_data_dir_for_profile(&profile_dir)
            .ok()
            .map(|dir| dir.to_string_lossy().to_string()),
    });

    // 上次运行如果没能清理干净，这里顺手补一次（与当前会话并行，不阻塞启动）。
    tauri::async_runtime::spawn_blocking(|| {
        log_cleanup_report(cleanup_stale_sessions());
    });

    let app_for_task = app.clone();
    let session_id_for_task = session_id.clone();
    tauri::async_runtime::spawn(async move {
        run_session(app_for_task, session_id_for_task, false).await;
    });

    logger::log_info(&format!(
        "[Codex临时登录] 会话已开始: session_id={}, profile_dir={}",
        session_id,
        profile_dir.display()
    ));
    Ok(CodexTempLoginSession {
        session_id,
        profile_dir: profile_dir.to_string_lossy().to_string(),
    })
}

/// 取消本次官方登录：关闭官方客户端并清理临时 profile。
pub fn cancel(session_id: &str) -> Result<(), String> {
    let sessions = lock_sessions();
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "官方登录会话不存在或已结束".to_string())?;
    if session.finished {
        return Ok(());
    }
    session.cancel.store(true, Ordering::SeqCst);
    logger::log_info(&format!(
        "[Codex临时登录] 收到取消请求: session_id={}",
        session_id
    ));
    Ok(())
}

/// Explicitly retry the final import of an owned, retained login session.
/// The frontend supplies an ID, never an arbitrary filesystem path.
pub fn retry_import(app: AppHandle, session_id: &str) -> Result<CodexTempLoginSession, String> {
    let id = uuid::Uuid::parse_str(session_id)
        .map_err(|_| "CODEX_TEMP_LOGIN_RECOVERY_NOT_FOUND")?
        .to_string();
    let profile_dir = temp_login_root_dir()?.join(&id);
    if !profile_dir
        .canonicalize()
        .map_err(|_| "CODEX_TEMP_LOGIN_RECOVERY_NOT_FOUND")?
        .starts_with(
            temp_login_root_dir()?
                .canonicalize()
                .map_err(|_| "CODEX_TEMP_LOGIN_RECOVERY_NOT_FOUND")?,
        )
    {
        return Err("CODEX_TEMP_LOGIN_RECOVERY_NOT_FOUND".to_string());
    }
    if !profile_dir.join(SESSION_MARKER_FILE_NAME).is_file() {
        return Err("CODEX_TEMP_LOGIN_RECOVERY_NOT_FOUND".to_string());
    }
    let mut sessions = lock_sessions();
    if sessions.values().any(|session| !session.finished) {
        return Err("CODEX_TEMP_LOGIN_BUSY".to_string());
    }
    sessions.insert(
        id.clone(),
        SessionRuntime {
            profile_dir: profile_dir.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            finished: false,
        },
    );
    drop(sessions);
    let task_id = id.clone();
    tauri::async_runtime::spawn(async move {
        run_session(app, task_id, true).await;
    });
    Ok(CodexTempLoginSession {
        session_id: id,
        profile_dir: profile_dir.to_string_lossy().to_string(),
    })
}

async fn run_session(app: AppHandle, session_id: String, recover: bool) {
    let mut outcome = if recover {
        SessionOutcome::CredentialsReady
    } else {
        run_session_flow(&app, &session_id).await
    };
    let Some(profile_dir) = session_profile_dir(&session_id) else {
        return;
    };

    emit_progress(&app, &session_id, "closing", 56, None, None);
    let close_error = close_temp_login_client(&profile_dir).await;
    if session_cancel_flag(&session_id).is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        outcome = SessionOutcome::Cancelled;
    }
    if matches!(outcome, SessionOutcome::CredentialsReady) {
        outcome = if let Some(error) = close_error.as_deref() {
            SessionOutcome::Failed(format!(
                "CODEX_TEMP_LOGIN_RECOVERY:{}|{}",
                profile_dir.display(),
                error
            ))
        } else {
            emit_progress(&app, &session_id, "importing", 76, None, None);
            match import_final_credentials(&profile_dir).await {
                Ok(account) => {
                    crate::commands::codex::reactivate_imported_current_if_needed(
                        std::slice::from_ref(&account),
                    )
                    .await;
                    SessionOutcome::Imported(account)
                }
                Err(error) => SessionOutcome::Failed(format!(
                    "CODEX_TEMP_LOGIN_RECOVERY:{}|{}",
                    profile_dir.display(),
                    error
                )),
            }
        };
    }
    // Never delete credentials while the client may still be writing, or before
    // the final snapshot has been imported successfully. Cancel explicitly discards it.
    let dir = profile_dir.clone();
    let discard = matches!(outcome, SessionOutcome::Cancelled);
    let preservation = tauri::async_runtime::spawn_blocking(move || {
        if discard {
            let marker_path = dir.join(SESSION_MARKER_FILE_NAME);
            let mut marker: serde_json::Value =
                serde_json::from_slice(&fs::read(&marker_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            marker["discardCredentials"] = serde_json::Value::Bool(true);
            fs::write(
                marker_path,
                serde_json::to_vec(&marker).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(official_credentials_present(&dir))
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    if discard {
        if let Err(error) = &preservation {
            outcome = SessionOutcome::Failed(error.clone());
        }
    }
    let preserve = !matches!(
        outcome,
        SessionOutcome::Imported(_) | SessionOutcome::Cancelled
    ) && preservation.unwrap_or(true);
    if preserve {
        if let SessionOutcome::Failed(error) = &mut outcome {
            if !error.starts_with("CODEX_TEMP_LOGIN_RECOVERY:") {
                *error = format!(
                    "CODEX_TEMP_LOGIN_RECOVERY:{}|{}",
                    profile_dir.display(),
                    error
                );
            }
        }
    }
    let cleanup_errors = if close_error.is_none() && !preserve {
        emit_progress(&app, &session_id, "cleaning", 88, None, None);
        let dir = profile_dir.clone();
        tauri::async_runtime::spawn_blocking(move || cleanup_profile_artifacts(&dir))
            .await
            .unwrap_or_else(|error| vec![error.to_string()])
    } else {
        Vec::new()
    };
    if close_error.is_none() && !preserve && cleanup_errors.is_empty() {
        unregister_pending_cleanup(&session_id);
    }

    match outcome {
        SessionOutcome::Imported(account) => {
            let cleanup_warning = close_error.or_else(|| cleanup_errors.first().cloned());
            emit_progress(
                &app,
                &session_id,
                "completed",
                100,
                Some(&account),
                cleanup_warning.as_deref(),
            );
            logger::log_info(&format!(
                "[Codex临时登录] 已导入账号: session_id={}, account_id={}, email={}",
                session_id, account.id, account.email
            ));
            // 额度/资料刷新放在后台，不阻塞登录结果回传。
            let app_for_refresh = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::codex::refresh_imported_codex_accounts(
                    &app_for_refresh,
                    vec![account],
                )
                .await;
            });
        }
        SessionOutcome::CredentialsReady => {
            unreachable!("final credentials must be imported before completion")
        }
        SessionOutcome::Cancelled => {
            let cleanup_warning = close_error.or_else(|| cleanup_errors.first().cloned());
            emit_progress(
                &app,
                &session_id,
                "cancelled",
                100,
                None,
                cleanup_warning.as_deref(),
            );
            logger::log_info(&format!(
                "[Codex临时登录] 已取消: session_id={}",
                session_id
            ));
        }
        SessionOutcome::Failed(error) => {
            emit_progress(&app, &session_id, "failed", 100, None, Some(error.as_str()));
            logger::log_warn(&format!(
                "[Codex临时登录] 登录失败: session_id={}, error={}",
                session_id, error
            ));
        }
    }

    mark_session_finished(&session_id);
    forget_session(&session_id);
}

/// Final import happens after the official runtime has stopped, off the async worker.
async fn import_final_credentials(profile_dir: &Path) -> Result<CodexAccount, String> {
    let mut last_error = String::new();
    for _ in 0..5 {
        let dir = profile_dir.to_path_buf();
        let result = tauri::async_runtime::spawn_blocking(move || {
            if !official_login_credentials_ready(&dir) {
                return Err("CODEX_TEMP_LOGIN_INCOMPLETE".to_string());
            }
            let account = codex_account::import_from_local_at(&dir)?;
            // Persist success before cleanup so an interrupted cleanup can safely resume.
            let marker_path = dir.join(SESSION_MARKER_FILE_NAME);
            let mut marker: serde_json::Value =
                serde_json::from_slice(&fs::read(&marker_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            marker["credentialsImported"] = serde_json::Value::Bool(true);
            fs::write(
                marker_path,
                serde_json::to_vec(&marker).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            Ok(account)
        })
        .await
        .map_err(|e| e.to_string())?;
        match result {
            Ok(account) => return Ok(account),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Err(last_error)
}

async fn launch_temp_login_client(profile_dir: &Path) -> Result<u32, String> {
    let dir = profile_dir.to_string_lossy().to_string();
    tauri::async_runtime::spawn_blocking(move || process::start_codex_with_args(&dir, &[]))
        .await
        .map_err(|e| e.to_string())?
}

async fn run_session_flow(app: &AppHandle, session_id: &str) -> SessionOutcome {
    let Some(profile_dir) = session_profile_dir(session_id) else {
        return SessionOutcome::Failed("官方登录会话不存在".to_string());
    };
    let Some(cancel) = session_cancel_flag(session_id) else {
        return SessionOutcome::Failed("官方登录会话不存在".to_string());
    };

    emit_progress(app, session_id, "preparing", 6, None, None);
    // start() has already created the blank profile and ownership marker.
    // Resetting it here would delete the marker needed for recovery and cleanup.
    emit_progress(app, session_id, "launching", 14, None, None);
    let client_pid = match launch_temp_login_client(&profile_dir).await {
        Ok(pid) => pid,
        Err(error) => return SessionOutcome::Failed(error),
    };
    logger::log_info(&format!(
        "[Codex临时登录] 官方客户端已启动（官方原生登录）: session_id={}, pid={}",
        session_id, client_pid
    ));
    emit_progress(app, session_id, "waiting-login", 24, None, None);
    let started_at = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return SessionOutcome::Cancelled;
        }
        if app_lifecycle::is_shutdown_started() {
            return SessionOutcome::Failed("应用正在退出，本次官方登录已中断".to_string());
        }
        if started_at.elapsed() >= LOGIN_TIMEOUT {
            return SessionOutcome::Failed(
                "等待官方客户端登录超时（10 分钟），本次登录已取消。".to_string(),
            );
        }

        let dir = profile_dir.clone();
        if tauri::async_runtime::spawn_blocking(move || official_login_credentials_ready(&dir))
            .await
            .unwrap_or(false)
        {
            return SessionOutcome::CredentialsReady;
        }
        if !process::is_pid_running(client_pid) {
            return SessionOutcome::Failed("CODEX_TEMP_LOGIN_CLOSED".to_string());
        }

        tokio::time::sleep(CREDENTIAL_POLL_INTERVAL).await;
    }
}

/// 以空白实例的方式准备临时 profile：目录必须为空，避免把其它登录状态带进来。
fn prepare_blank_profile_dir(profile_dir: &Path) -> Result<(), String> {
    if profile_dir.exists() {
        let has_entries = fs::read_dir(profile_dir)
            .map_err(|error| {
                format!(
                    "读取临时配置目录失败 ({}): {}",
                    profile_dir.display(),
                    error
                )
            })?
            .next()
            .is_some();
        if has_entries {
            fs::remove_dir_all(profile_dir).map_err(|error| {
                format!(
                    "重置临时配置目录失败 ({}): {}",
                    profile_dir.display(),
                    error
                )
            })?;
        }
    }
    fs::create_dir_all(profile_dir).map_err(|error| {
        format!(
            "创建临时配置目录失败 ({}): {}",
            profile_dir.display(),
            error
        )
    })
}

fn write_session_marker(profile_dir: &Path, session_id: &str) -> Result<(), String> {
    let marker = serde_json::json!({
        "sessionId": session_id,
        "createdAt": chrono::Utc::now().timestamp_millis(),
    });
    fs::write(
        profile_dir.join(SESSION_MARKER_FILE_NAME),
        serde_json::to_string_pretty(&marker).unwrap_or_default(),
    )
    .map_err(|error| format!("写入临时登录标记失败: {}", error))
}

/// 注入脚本与采集文件固定放在临时 profile 内，随会话一起硬删除。
/// 处理一条采集记录：把官方授权地址（或注入失败原因）回传前端。
/// 官方客户端是否已经把登录信息落到这个临时 profile。
///
/// 探测只使用低成本信号：`auth.json` 已出现凭据字段，或 macOS 钥匙串里已有该目录
/// 对应的条目（只查条目是否存在，不读取密钥，因此不会弹授权框）。
fn official_credentials_present(profile_dir: &Path) -> bool {
    profile_dir.join("secrets/codex_auth.age").is_file()
        || auth_file_has_credentials(profile_dir)
        || codex_account::codex_keychain_entry_exists_for_dir(profile_dir)
}

fn official_login_credentials_ready(profile_dir: &Path) -> bool {
    codex_account::read_official_oauth_tokens(profile_dir).is_some_and(|tokens| {
        !tokens.id_token.trim().is_empty()
            && !tokens.access_token.trim().is_empty()
            && tokens
                .refresh_token
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty())
    })
}

fn credentials_need_recovery(profile_dir: &Path) -> bool {
    let imported = fs::read(profile_dir.join(SESSION_MARKER_FILE_NAME))
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .is_some_and(|marker| {
            marker["credentialsImported"] == true || marker["discardCredentials"] == true
        });
    !imported && official_credentials_present(profile_dir)
}

fn auth_file_has_credentials(profile_dir: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(profile_dir.join("auth.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        // 官方客户端可能正在写入，解析失败时继续等待。
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    [
        "tokens",
        "OPENAI_API_KEY",
        "personal_access_token",
        "agent_identity",
        "agentIdentity",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}

async fn close_temp_login_client(profile_dir: &Path) -> Option<String> {
    let target = profile_dir.to_string_lossy().to_string();
    match tauri::async_runtime::spawn_blocking(move || {
        process::close_codex_instances(std::slice::from_ref(&target), CLOSE_TIMEOUT_SECS)
    })
    .await
    {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(error) => Some(format!("关闭官方客户端任务执行失败: {}", error)),
    }
}

/// 销毁临时 profile 的全部痕迹：Electron 运行目录、钥匙串条目、配置目录。
///
/// 这里必须硬删除（不能走「移到回收站」），否则登录凭据会以副本形式留在本机。
fn cleanup_profile_artifacts(profile_dir: &Path) -> Vec<String> {
    let app_user_data_dir = app_user_data_dir_for_profile(profile_dir).ok();
    cleanup_profile_artifacts_with_runtime_dir(profile_dir, app_user_data_dir.as_deref())
}

/// 与 [`cleanup_profile_artifacts`] 相同，但运行目录由调用方给出。
///
/// 待清理记录里保存的是注册时解析好的运行目录：profile 目录被删除后
/// `canonicalize` 会失败，重新推导可能得到不同的路径，因此必须按记录删除。
fn cleanup_profile_artifacts_with_runtime_dir(
    profile_dir: &Path,
    app_user_data_dir: Option<&Path>,
) -> Vec<String> {
    let mut errors = Vec::new();

    if let Some(app_user_data_dir) = app_user_data_dir {
        if app_user_data_dir.exists() {
            if let Err(error) = fs::remove_dir_all(app_user_data_dir) {
                errors.push(format!(
                    "删除临时实例运行目录失败 ({}): {}",
                    app_user_data_dir.display(),
                    error
                ));
            }
        }
    }

    if let Err(error) = codex_account::delete_codex_keychain_entry_for_dir(profile_dir) {
        errors.push(error);
        // Keep the canonical home path for retrying its keyring entry identifier.
        return errors;
    }

    if profile_dir.exists() {
        if let Err(error) = fs::remove_dir_all(profile_dir) {
            errors.push(format!(
                "删除临时配置目录失败 ({}): {}",
                profile_dir.display(),
                error
            ));
        }
    }

    errors
}

fn app_user_data_dir_for_profile(profile_dir: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        return codex_instance::get_macos_app_user_data_dir(profile_dir);
    }
    #[cfg(target_os = "windows")]
    {
        return codex_instance::get_windows_app_user_data_dir(profile_dir);
    }
    #[cfg(target_os = "linux")]
    {
        return codex_instance::get_linux_app_user_data_dir(profile_dir);
    }
    #[allow(unreachable_code)]
    {
        let _ = profile_dir;
        Err("当前平台不支持 Codex 实例运行目录".to_string())
    }
}

/// 清理所有残留的临时登录 profile（含本次进程之外的遗留目录）。
///
/// 先按 CODEX_HOME 关闭仍在使用该 profile 的官方客户端，再删除目录与钥匙串条目；
/// 关闭失败时不删除目录，避免留下半份数据，等下次巡检重试。
pub fn cleanup_stale_sessions() -> CodexTempLoginCleanupReport {
    let mut report = CodexTempLoginCleanupReport::default();
    let root = match temp_login_root_dir() {
        Ok(root) => root,
        Err(error) => {
            report.failed.push(CodexTempLoginCleanupFailure {
                path: TEMP_LOGIN_ROOT_DIR_NAME.to_string(),
                error,
            });
            return report;
        }
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return report;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == PENDING_CLEANUP_FILE_NAME)
        {
            // 待清理清单属于巡检自身的状态文件，不能当作残留物删除。
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if !is_dir {
            if fs::remove_file(&path).is_ok() {
                report.removed.push(path.to_string_lossy().to_string());
            }
            continue;
        }
        if is_active_session_dir(&path) || credentials_need_recovery(&path) {
            continue;
        }

        let target = path.to_string_lossy().to_string();
        if let Err(error) =
            process::close_codex_instances(std::slice::from_ref(&target), CLOSE_TIMEOUT_SECS)
        {
            report.failed.push(CodexTempLoginCleanupFailure {
                path: target,
                error: format!("关闭官方客户端失败: {}", error),
            });
            continue;
        }

        let errors = cleanup_profile_artifacts(&path);
        if errors.is_empty() {
            report.removed.push(target);
        } else {
            for error in errors {
                report.failed.push(CodexTempLoginCleanupFailure {
                    path: target.clone(),
                    error,
                });
            }
        }
    }

    retry_pending_cleanup(&mut report);
    report
}

/// 按待清理记录重试：覆盖「profile 目录已删除但运行目录/钥匙串条目还在」的情况。
fn retry_pending_cleanup(report: &mut CodexTempLoginCleanupReport) {
    let mut pending = read_pending_cleanup_entries();
    if pending.is_empty() {
        return;
    }

    let mut survivors = Vec::new();
    for entry in pending.drain(..) {
        if is_active_session_id(&entry.session_id) {
            survivors.push(entry);
            continue;
        }

        let profile_dir = PathBuf::from(&entry.profile_dir);
        if credentials_need_recovery(&profile_dir) {
            survivors.push(entry);
            continue;
        }
        let app_user_data_dir = entry.app_user_data_dir.as_deref().map(PathBuf::from);
        let app_user_data_exists = app_user_data_dir.as_ref().is_some_and(|dir| dir.exists());
        if !profile_dir.exists() && !app_user_data_exists {
            // 已经没有残留物，直接撤销记录。
            continue;
        }

        if profile_dir.exists() {
            let target = entry.profile_dir.clone();
            if let Err(error) =
                process::close_codex_instances(std::slice::from_ref(&target), CLOSE_TIMEOUT_SECS)
            {
                report.failed.push(CodexTempLoginCleanupFailure {
                    path: entry.profile_dir.clone(),
                    error: format!("关闭官方客户端失败: {}", error),
                });
                survivors.push(entry);
                continue;
            }
        }

        let mut errors =
            cleanup_profile_artifacts_with_runtime_dir(&profile_dir, app_user_data_dir.as_deref());
        if errors.is_empty() {
            report.removed.push(entry.profile_dir.clone());
        } else {
            for error in errors.drain(..) {
                report.failed.push(CodexTempLoginCleanupFailure {
                    path: entry.profile_dir.clone(),
                    error,
                });
            }
            survivors.push(entry);
        }
    }

    pending = survivors;
    write_pending_cleanup_entries(&pending);
}

fn is_active_session_id(session_id: &str) -> bool {
    lock_sessions()
        .get(session_id)
        .is_some_and(|session| !session.finished)
}

fn log_cleanup_report(report: CodexTempLoginCleanupReport) {
    if report.removed.is_empty() && report.failed.is_empty() {
        return;
    }
    logger::log_info(&format!(
        "[Codex临时登录] 残留巡检完成: removed={}, failed={}",
        report.removed.len(),
        report.failed.len()
    ));
    for failure in report.failed {
        logger::log_warn(&format!(
            "[Codex临时登录] 残留清理失败: path={}, error={}",
            failure.path, failure.error
        ));
    }
}

/// 启动定期巡检：清理历史遗留的临时登录 profile。
pub fn ensure_cleanup_loop_started() {
    if CLEANUP_LOOP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    logger::log_info("[Codex临时登录] 残留清理巡检已启动");
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CLEANUP_STARTUP_DELAY).await;
        loop {
            if app_lifecycle::is_shutdown_started() {
                return;
            }
            match tauri::async_runtime::spawn_blocking(cleanup_stale_sessions).await {
                Ok(report) => log_cleanup_report(report),
                Err(error) => {
                    logger::log_warn(&format!("[Codex临时登录] 残留巡检任务执行失败: {}", error))
                }
            }
            tokio::time::sleep(CLEANUP_SWEEP_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_import_uses_the_last_persisted_credentials_before_cleanup() {
        use base64::Engine;
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-final-import");
        let dir = env.root.join("profile");
        prepare_blank_profile_dir(&dir).unwrap();
        write_session_marker(&dir, "session").unwrap();
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::json!({"email":"final@example.test","sub":"synthetic-user", "exp":4102444800_i64,
                "https://api.openai.com/auth":{"chatgpt_account_id":"synthetic-account"}}).to_string(),
        );
        let mut payload = serde_json::json!({"auth_mode":"chatgpt", "tokens":{
            "id_token":format!("e30.{claims}.signature"), "access_token":format!("e30.{claims}.signature"),
            "refresh_token":"old-refresh", "account_id":"synthetic-account"}});
        fs::write(dir.join("auth.json"), payload.to_string()).unwrap();
        assert!(official_login_credentials_ready(&dir));
        // Simulate the official runtime rotating the token before it exits.
        payload["tokens"]["refresh_token"] = serde_json::json!("final-refresh");
        fs::write(dir.join("auth.json"), payload.to_string()).unwrap();
        let account = tauri::async_runtime::block_on(import_final_credentials(&dir)).unwrap();
        assert_eq!(
            account.tokens.refresh_token.as_deref(),
            Some("final-refresh")
        );
        assert_eq!(account.email, "final@example.test");
        assert!(!credentials_need_recovery(&dir));
        assert!(dir.join("auth.json").is_file());
        assert!(cleanup_profile_artifacts(&dir).is_empty());
        assert!(!dir.exists());
    }

    #[test]
    fn recovery_keeps_unimported_credentials_and_allows_cleanup_after_import() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-recovery");
        let dir = env.root.join("profile");
        prepare_blank_profile_dir(&dir).unwrap();
        write_session_marker(&dir, "session").unwrap();
        fs::write(
            dir.join("auth.json"),
            r#"{"tokens":{"access_token":"test","id_token":"test"}}"#,
        )
        .unwrap();
        assert!(credentials_need_recovery(&dir));
        assert!(!official_login_credentials_ready(&dir));
        fs::write(
            dir.join(SESSION_MARKER_FILE_NAME),
            r#"{"discardCredentials":true}"#,
        )
        .unwrap();
        assert!(!credentials_need_recovery(&dir));
        fs::write(
            dir.join(SESSION_MARKER_FILE_NAME),
            r#"{"credentialsImported":true}"#,
        )
        .unwrap();
        assert!(!credentials_need_recovery(&dir));
    }

    #[test]
    fn encrypted_credentials_are_preserved_for_recovery_without_decryption() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-encrypted-recovery");
        let dir = env.root.join("profile");
        fs::create_dir_all(dir.join("secrets")).unwrap();
        fs::write(dir.join("secrets/codex_auth.age"), "encrypted").unwrap();
        assert!(official_credentials_present(&dir));
        assert!(credentials_need_recovery(&dir));
    }

    /// 隔离测试数据目录与 HOME，避免测试写入真实应用数据。
    struct TestEnvGuard {
        root: PathBuf,
        previous_data_dir: Option<String>,
        previous_home: Option<String>,
    }

    impl TestEnvGuard {
        fn new(prefix: &str) -> Self {
            let root = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).expect("create test root");
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").ok();
            let previous_home = std::env::var("HOME").ok();
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &root);
            std::env::set_var("HOME", &root);
            Self {
                root,
                previous_data_dir,
                previous_home,
            }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match self.previous_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            match self.previous_home.as_ref() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn blank_profile_dir_is_created_and_reset_when_not_empty() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-blank-profile");
        let profile_dir = env.root.join("profile");

        prepare_blank_profile_dir(&profile_dir).expect("create blank profile");
        assert!(profile_dir.is_dir());

        // 与多开实例的空白实例一致：已有内容必须在开始前清空，避免带进旧登录状态。
        fs::write(profile_dir.join("auth.json"), "{\"tokens\":{}}").expect("write stale auth");
        prepare_blank_profile_dir(&profile_dir).expect("reset blank profile");
        assert!(!profile_dir.join("auth.json").exists());
    }

    #[test]
    fn credentials_detection_requires_a_complete_credential_payload() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-credentials");
        let profile_dir = env.root.join("profile");
        fs::create_dir_all(&profile_dir).expect("create profile dir");

        assert!(!official_credentials_present(&profile_dir));

        // 官方客户端写入中的半截文件不能被当成登录完成。
        fs::write(profile_dir.join("auth.json"), "{\"tokens\":").expect("write partial file");
        assert!(!official_credentials_present(&profile_dir));

        fs::write(profile_dir.join("auth.json"), "{\"auth_mode\":\"chatgpt\"}")
            .expect("write placeholder file");
        assert!(!official_credentials_present(&profile_dir));

        fs::write(
            profile_dir.join("auth.json"),
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": { "access_token": "token", "id_token": "id" },
            })
            .to_string(),
        )
        .expect("write credential file");
        assert!(official_credentials_present(&profile_dir));
    }

    #[test]
    fn cleanup_removes_artifacts_and_reports_no_errors() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-cleanup");
        let profile_dir = env.root.join("codex-temp-login").join("session-a");
        prepare_blank_profile_dir(&profile_dir).expect("create blank profile");
        write_session_marker(&profile_dir, "session-a").expect("write marker");

        let errors = cleanup_profile_artifacts(&profile_dir);
        assert!(errors.is_empty(), "unexpected cleanup errors: {:?}", errors);
        assert!(!profile_dir.exists());
    }

    #[test]
    fn stale_sweep_removes_leftovers_but_keeps_the_active_session() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-sweep");
        let root = temp_login_root_dir().expect("resolve temp login root");
        let active_dir = root.join("active-session");
        let leftover_dir = root.join("leftover-session");
        prepare_blank_profile_dir(&active_dir).expect("create active profile");
        prepare_blank_profile_dir(&leftover_dir).expect("create leftover profile");

        lock_sessions().insert(
            "active-session".to_string(),
            SessionRuntime {
                profile_dir: active_dir.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                finished: false,
            },
        );

        let report = cleanup_stale_sessions();
        lock_sessions().remove("active-session");

        assert!(!leftover_dir.exists(), "leftover session should be removed");
        assert!(active_dir.exists(), "active session must not be removed");
        assert!(
            report
                .removed
                .iter()
                .any(|path| path.ends_with("leftover-session")),
            "cleanup report should list the removed directory: {:?}",
            report.removed
        );
    }

    #[test]
    fn pending_cleanup_retries_orphaned_runtime_dir_after_profile_was_removed() {
        let _lock = lock_env();
        let env = TestEnvGuard::new("codex-temp-login-pending");
        let root = temp_login_root_dir().expect("resolve temp login root");
        let profile_dir = root.join("partial-session");
        prepare_blank_profile_dir(&profile_dir).expect("create profile");

        // 模拟「profile 目录删除成功、运行目录删除失败」的半清理状态。
        let runtime_dir = env
            .root
            .join("instances")
            .join("codex-app-data")
            .join("hash");
        fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        register_pending_cleanup(PendingCleanupEntry {
            session_id: "partial-session".to_string(),
            profile_dir: profile_dir.to_string_lossy().to_string(),
            app_user_data_dir: Some(runtime_dir.to_string_lossy().to_string()),
        });
        fs::remove_dir_all(&profile_dir).expect("remove profile dir");

        cleanup_stale_sessions();

        assert!(
            !runtime_dir.exists(),
            "orphaned runtime dir should be removed by the sweep"
        );
        assert!(
            read_pending_cleanup_entries().is_empty(),
            "pending entry should be cleared once cleanup succeeds"
        );
    }
}
