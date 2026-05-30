use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::models::zed::ZedRuntimeStatus;
use crate::modules::{account, logger, process, zed_account};

static ZED_RUNTIME_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const ZED_RUNTIME_FILE: &str = "zed_runtime.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ZedRuntimeState {
    #[serde(default)]
    last_pid: Option<u32>,
    #[serde(default)]
    last_started_at: Option<i64>,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn runtime_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(ZED_RUNTIME_FILE))
}

fn load_runtime_state() -> ZedRuntimeState {
    let path = match runtime_path() {
        Ok(path) => path,
        Err(_) => return ZedRuntimeState::default(),
    };
    if !path.exists() {
        return ZedRuntimeState::default();
    }
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(error) => {
                match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                    Ok(Some(backup_path)) => logger::log_warn(&format!(
                        "[Zed] 运行时状态解析失败，已隔离并使用空状态: path={}, backup={}, error={}",
                        path.display(),
                        backup_path.display(),
                        error
                    )),
                    Ok(None) => logger::log_warn(&format!(
                        "[Zed] 运行时状态解析失败，文件已不存在，使用空状态: path={}, error={}",
                        path.display(),
                        error
                    )),
                    Err(backup_error) => logger::log_warn(&format!(
                        "[Zed] 运行时状态解析失败，隔离失败，使用空状态: path={}, parse_error={}, backup_error={}",
                        path.display(),
                        error,
                        backup_error
                    )),
                }
                ZedRuntimeState::default()
            }
        },
        Err(_) => ZedRuntimeState::default(),
    }
}

fn save_runtime_state(state: &ZedRuntimeState) -> Result<(), String> {
    let path = runtime_path()?;
    let content = serde_json::to_string_pretty(state)
        .map_err(|e| format!("序列化 Zed 运行时状态失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存 Zed 运行时状态失败: {}", e))
}

fn resolve_running_pid(last_pid: Option<u32>) -> Option<u32> {
    let pid = last_pid?;
    if process::is_pid_running(pid) {
        Some(pid)
    } else {
        None
    }
}

fn update_runtime_state(last_pid: Option<u32>, last_started_at: Option<i64>) -> Result<(), String> {
    let _lock = ZED_RUNTIME_LOCK
        .lock()
        .map_err(|_| "获取 Zed 运行时锁失败".to_string())?;
    let mut state = load_runtime_state();
    state.last_pid = last_pid;
    state.last_started_at = last_started_at;
    save_runtime_state(&state)
}

fn app_path_configured() -> bool {
    process::resolve_zed_launch_path().is_ok()
}

fn build_runtime_status(state: ZedRuntimeState) -> ZedRuntimeStatus {
    let running_pid = resolve_running_pid(state.last_pid);
    ZedRuntimeStatus {
        running: running_pid.is_some(),
        last_pid: running_pid,
        last_started_at: state.last_started_at,
        current_account_id: zed_account::resolve_current_account_id(),
        app_path_configured: app_path_configured(),
    }
}

pub fn get_runtime_status() -> ZedRuntimeStatus {
    build_runtime_status(load_runtime_state())
}

fn build_launch_command(launch_path: &std::path::Path) -> Command {
    let mut command = Command::new(launch_path);
    process::apply_managed_proxy_env_to_command(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }

    command
}

pub fn start_default_session() -> Result<ZedRuntimeStatus, String> {
    process::ensure_zed_launch_path_configured()?;

    let current = get_runtime_status();
    if current.running {
        return Ok(current);
    }

    let launch_path = process::resolve_zed_launch_path()?;
    let mut command = build_launch_command(&launch_path);

    let child = command
        .spawn()
        .map_err(|e| format!("启动 Zed 失败: {}", e))?;
    let pid = child.id();

    update_runtime_state(Some(pid), Some(now_ts()))?;
    logger::log_info(&format!(
        "[Zed] 已启动默认会话: launch_path={}, pid={}",
        launch_path.display(),
        pid
    ));
    Ok(get_runtime_status())
}

#[cfg(target_os = "macos")]
fn quit_zed_app() {
    let _ = Command::new("osascript")
        .args(["-e", "tell application \"Zed\" to quit"])
        .output();
}

#[cfg(not(target_os = "macos"))]
fn quit_zed_app() {}

pub fn stop_default_session() -> Result<ZedRuntimeStatus, String> {
    let state = load_runtime_state();

    #[cfg(target_os = "macos")]
    {
        quit_zed_app();
        thread::sleep(Duration::from_millis(1200));
    }

    if let Some(pid) = resolve_running_pid(state.last_pid) {
        process::close_pid(pid, 20)?;
    }

    update_runtime_state(None, state.last_started_at)?;
    logger::log_info("[Zed] 已停止默认会话");
    Ok(get_runtime_status())
}

pub fn restart_default_session() -> Result<ZedRuntimeStatus, String> {
    let previous = load_runtime_state();
    #[cfg(target_os = "macos")]
    {
        quit_zed_app();
        thread::sleep(Duration::from_millis(1200));
    }
    if let Some(pid) = resolve_running_pid(previous.last_pid) {
        process::close_pid(pid, 20)?;
    }
    update_runtime_state(None, previous.last_started_at)?;
    thread::sleep(Duration::from_millis(400));
    start_default_session()
}

pub fn focus_default_session() -> Result<ZedRuntimeStatus, String> {
    let state = load_runtime_state();
    let pid = resolve_running_pid(state.last_pid).ok_or_else(|| "Zed 未运行".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .args(["-e", "tell application \"Zed\" to activate"])
            .output()
            .map_err(|e| format!("执行 Zed activate 失败: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "激活 Zed 失败: status={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        process::focus_process_pid(pid).map_err(|e| format!("聚焦 Zed 窗口失败: {}", e))?;
    }

    logger::log_info(&format!("[Zed] 已聚焦默认会话: pid={}", pid));
    Ok(get_runtime_status())
}
